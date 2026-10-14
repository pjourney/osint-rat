//! Local geo lookups. No network: JSON or MMDB files on disk, plus country centroids.

use std::fs;
use std::net::IpAddr;
use std::path::Path;
use std::str::FromStr;

use crate::model::{
    GeoDerivation, GeoLocation, GeoPoint, parse_ip, parse_latitude_json, parse_longitude_json,
};

/// Looks up an IP in an optional local database. Missing/unreadable files degrade to `None`.
pub fn lookup_ip(path: Option<&Path>, ip: IpAddr) -> GeoLookup {
    let Some(path) = path else {
        return GeoLookup {
            location: None,
            warning: None,
        };
    };
    if !path.exists() {
        return GeoLookup {
            location: None,
            warning: Some(format!(
                "geo database {} is missing; continuing without coordinates",
                path.display()
            )),
        };
    }
    match load(path, ip) {
        Ok(location) => GeoLookup {
            location,
            warning: None,
        },
        Err(message) => GeoLookup {
            location: None,
            warning: Some(message),
        },
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GeoLookup {
    pub location: Option<GeoLocation>,
    pub warning: Option<String>,
}

fn load(path: &Path, ip: IpAddr) -> Result<Option<GeoLocation>, String> {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if extension == "mmdb" {
        return load_mmdb(path, ip);
    }
    load_json(path, ip)
}

fn load_json(path: &Path, ip: IpAddr) -> Result<Option<GeoLocation>, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("could not read geo database {}: {error}", path.display()))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("geo database {} is not valid JSON: {error}", path.display()))?;
    let records = value
        .get("records")
        .and_then(serde_json::Value::as_array)
        .or_else(|| value.as_array())
        .ok_or_else(|| format!("geo database {} has no records array", path.display()))?;
    for record in records {
        if record_matches(record, ip) {
            return Ok(Some(location_from_json(record)));
        }
    }
    Ok(None)
}

fn record_matches(record: &serde_json::Value, ip: IpAddr) -> bool {
    if let Some(exact) = record
        .get("ip")
        .and_then(serde_json::Value::as_str)
        .and_then(parse_ip)
    {
        return exact == ip;
    }
    let Some(network) = record.get("network").and_then(serde_json::Value::as_str) else {
        return false;
    };
    prefix_contains(network, ip)
}

fn prefix_contains(network: &str, ip: IpAddr) -> bool {
    let Some((base, bits)) = network.split_once('/') else {
        return parse_ip(network) == Some(ip);
    };
    let Ok(base_ip) = IpAddr::from_str(base.trim()) else {
        return false;
    };
    let Ok(len) = bits.trim().parse::<u32>() else {
        return false;
    };
    match (base_ip, ip) {
        (IpAddr::V4(base), IpAddr::V4(addr)) => {
            if len > 32 {
                return false;
            }
            let mask = if len == 0 { 0 } else { u32::MAX << (32 - len) };
            (u32::from(base) & mask) == (u32::from(addr) & mask)
        }
        (IpAddr::V6(base), IpAddr::V6(addr)) => {
            if len > 128 {
                return false;
            }
            let mask = if len == 0 {
                0
            } else {
                u128::MAX << (128 - len)
            };
            (u128::from(base) & mask) == (u128::from(addr) & mask)
        }
        _ => false,
    }
}

fn location_from_json(record: &serde_json::Value) -> GeoLocation {
    let country = record
        .get("country")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let latitude = record
        .get("latitude")
        .and_then(parse_latitude_json)
        .or_else(|| record.get("lat").and_then(parse_latitude_json));
    let longitude = record
        .get("longitude")
        .and_then(parse_longitude_json)
        .or_else(|| record.get("lon").and_then(parse_longitude_json));
    let point = match (latitude, longitude) {
        (Some(latitude), Some(longitude)) => GeoPoint::new(latitude, longitude),
        _ => None,
    };
    GeoLocation {
        country,
        point,
        prefix: record
            .get("network")
            .and_then(serde_json::Value::as_str)
            .map(ToString::to_string),
        derivation: GeoDerivation::GeoDb,
    }
}

fn load_mmdb(path: &Path, ip: IpAddr) -> Result<Option<GeoLocation>, String> {
    let reader = maxminddb::Reader::open_readfile(path)
        .map_err(|error| format!("could not open geo database {}: {error}", path.display()))?;
    let lookup: Result<Option<maxminddb::geoip2::City<'_>>, _> = reader.lookup(ip);
    let city = match lookup {
        Ok(value) => value,
        Err(error) => {
            return Err(format!(
                "geo database {} lookup failed: {error}",
                path.display()
            ));
        }
    };
    let Some(city) = city else {
        return Ok(None);
    };
    let country = city
        .country
        .as_ref()
        .and_then(|country| country.iso_code)
        .map(ToString::to_string);
    let point = city.location.as_ref().and_then(|location| {
        GeoPoint::new(
            location.latitude.unwrap_or(f64::NAN),
            location.longitude.unwrap_or(f64::NAN),
        )
    });
    if country.is_none() && point.is_none() {
        return Ok(None);
    }
    Ok(Some(GeoLocation {
        country,
        point,
        prefix: None,
        derivation: GeoDerivation::GeoDb,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn missing_database_degrades_without_panic() {
        let actual = lookup_ip(
            Some(Path::new("/no/such/geo.mmdb")),
            "192.0.2.1".parse().unwrap(),
        );
        assert!(actual.location.is_none());
        assert!(actual.warning.as_deref().unwrap_or("").contains("missing"));
        let none = lookup_ip(None, "192.0.2.1".parse().unwrap());
        assert!(none.location.is_none());
        assert!(none.warning.is_none());
    }

    #[test]
    fn fixture_database_is_used_when_present() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/geo/city.json");
        let hit = lookup_ip(Some(&path), "192.0.2.77".parse().unwrap());
        let location = hit.location.expect("fixture hit");
        assert_eq!(location.country.as_deref(), Some("US"));
        assert_eq!(location.point, GeoPoint::new(38.0, -97.0));
        let refused = lookup_ip(Some(&path), "198.51.100.9".parse().unwrap());
        assert!(refused.location.expect("row").point.is_none());
    }

    #[test]
    fn json_prefix_lookup_returns_coordinates() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("geo.json");
        let mut file = fs::File::create(&path).expect("create");
        file.write_all(
            br#"{
              "records": [
                {"network": "192.0.2.0/24", "country": "US", "latitude": 38.0, "longitude": -97.0},
                {"network": "198.51.100.0/24", "country": "AU", "latitude": "NaN", "longitude": 133.0}
              ]
            }"#,
        )
        .expect("write");
        let hit = lookup_ip(Some(&path), "192.0.2.77".parse().unwrap());
        let location = hit.location.expect("hit");
        assert_eq!(location.country.as_deref(), Some("US"));
        assert_eq!(location.point, GeoPoint::new(38.0, -97.0));
        let refused = lookup_ip(Some(&path), "198.51.100.9".parse().unwrap());
        assert!(refused.location.expect("row").point.is_none());
        let miss = lookup_ip(Some(&path), "203.0.113.1".parse().unwrap());
        assert!(miss.location.is_none());
    }
}
