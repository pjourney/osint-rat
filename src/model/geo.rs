//! Coordinates, GeoJSON, and nearby matching. Parsing never panics on untrusted input.

use std::net::IpAddr;

use serde::Serialize;

use super::common::{Fields, Provenance};
use super::entity::{Entity, EntityKind};
use super::workspace::InvestigationData;

pub const MIN_LATITUDE: f64 = -90.0;
pub const MAX_LATITUDE: f64 = 90.0;
pub const MIN_LONGITUDE: f64 = -180.0;
pub const MAX_LONGITUDE: f64 = 180.0;

/// A validated WGS-84 point. Longitude is first in GeoJSON coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeoPoint {
    pub latitude: f64,
    pub longitude: f64,
}

impl GeoPoint {
    pub fn new(latitude: f64, longitude: f64) -> Option<Self> {
        if !is_valid_latitude(latitude) || !is_valid_longitude(longitude) {
            return None;
        }
        Some(Self {
            latitude,
            longitude,
        })
    }

    pub fn coordinates(self) -> [f64; 2] {
        [self.longitude, self.latitude]
    }
}

/// How a lat/lon or country value was produced. Stored on the entity field bag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeoDerivation {
    Collected,
    GeoDb,
    CountryCentroid,
}

impl GeoDerivation {
    pub fn as_str(self) -> &'static str {
        match self {
            GeoDerivation::Collected => "collected",
            GeoDerivation::GeoDb => "geo_db",
            GeoDerivation::CountryCentroid => "country_centroid",
        }
    }
}

/// A local lookup result. Built-ins never fetch a geo service.
#[derive(Clone, Debug, PartialEq)]
pub struct GeoLocation {
    pub country: Option<String>,
    pub point: Option<GeoPoint>,
    pub prefix: Option<String>,
    pub derivation: GeoDerivation,
}

/// Parses an untrusted latitude. `NaN`, infinities, and values outside ±90 are refused.
pub fn parse_latitude(raw: &str) -> Option<f64> {
    parse_finite(raw).filter(|value| is_valid_latitude(*value))
}

/// Parses an untrusted longitude. `NaN`, infinities, and values outside ±180 are refused.
pub fn parse_longitude(raw: &str) -> Option<f64> {
    parse_finite(raw).filter(|value| is_valid_longitude(*value))
}

pub fn parse_latitude_json(value: &serde_json::Value) -> Option<f64> {
    json_finite(value).filter(|number| is_valid_latitude(*number))
}

pub fn parse_longitude_json(value: &serde_json::Value) -> Option<f64> {
    json_finite(value).filter(|number| is_valid_longitude(*number))
}

pub fn is_valid_latitude(value: f64) -> bool {
    value.is_finite() && (MIN_LATITUDE..=MAX_LATITUDE).contains(&value)
}

pub fn is_valid_longitude(value: f64) -> bool {
    value.is_finite() && (MIN_LONGITUDE..=MAX_LONGITUDE).contains(&value)
}

fn parse_finite(raw: &str) -> Option<f64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || is_non_number(trimmed) {
        return None;
    }
    let value: f64 = trimmed.parse().ok()?;
    value.is_finite().then_some(value)
}

fn json_finite(value: &serde_json::Value) -> Option<f64> {
    match value {
        serde_json::Value::Number(number) => number.as_f64().filter(|value| value.is_finite()),
        serde_json::Value::String(text) => parse_finite(text),
        _ => None,
    }
}

fn is_non_number(raw: &str) -> bool {
    matches!(
        raw.to_ascii_lowercase().as_str(),
        "nan"
            | "+nan"
            | "-nan"
            | "inf"
            | "+inf"
            | "-inf"
            | "infinity"
            | "+infinity"
            | "-infinity"
            | "null"
            | "none"
    )
}

/// Reads a validated point from an entity property bag.
pub fn point_from_fields(fields: &Fields) -> Option<GeoPoint> {
    let latitude = fields.get("latitude").and_then(parse_latitude)?;
    let longitude = fields.get("longitude").and_then(parse_longitude)?;
    GeoPoint::new(latitude, longitude)
}

/// Approximate country centroid used only when a collector recorded a country but no
/// coordinates. This is derived data (`geo_source=country_centroid`); it is not a survey.
pub fn country_centroid(country: &str) -> Option<GeoPoint> {
    let key = normalize_country(country)?;
    CENTROIDS
        .iter()
        .find(|(code, _, _)| *code == key)
        .and_then(|(_, latitude, longitude)| GeoPoint::new(*latitude, *longitude))
}

pub fn normalize_country(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || is_non_number(trimmed) {
        return None;
    }
    let upper = trimmed.to_ascii_uppercase();
    if upper.len() == 2 && upper.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return Some(upper);
    }
    ALIASES
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(trimmed))
        .map(|(_, code)| (*code).to_string())
}

/// Entities that share a country or announced prefix with `seed`.
pub fn nearby_entities<'a>(data: &'a InvestigationData, seed: &Entity) -> Vec<&'a Entity> {
    let country = seed.fields.get("country").and_then(normalize_country);
    let prefix = seed
        .fields
        .get("prefix")
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if country.is_none() && prefix.is_none() {
        return Vec::new();
    }
    let canonical_seed = data.canonical_id(seed.id);
    data.entities
        .iter()
        .filter(|entity| {
            let canonical = data.canonical_id(entity.id);
            if canonical == canonical_seed {
                return false;
            }
            if data.is_absorbed(entity.id) {
                return false;
            }
            let same_country = match (country.as_deref(), entity.fields.get("country")) {
                (Some(left), Some(right)) => normalize_country(right).as_deref() == Some(left),
                _ => false,
            };
            let same_prefix = match (prefix, entity.fields.get("prefix")) {
                (Some(left), Some(right)) => left == right.trim(),
                _ => false,
            };
            same_country || same_prefix
        })
        .collect()
}

/// GeoJSON FeatureCollection for entities that have a point, a country, or a prefix.
/// Each located entity contributes a Point and a small envelope Polygon plus properties.
pub fn geojson_collection(data: &InvestigationData) -> GeoJsonFeatureCollection {
    let mut features = Vec::new();
    for entity in &data.entities {
        if data.is_absorbed(entity.id) {
            continue;
        }
        features.extend(features_for_entity(data, entity));
    }
    GeoJsonFeatureCollection {
        feature_type: "FeatureCollection",
        features,
    }
}

fn features_for_entity(data: &InvestigationData, entity: &Entity) -> Vec<GeoJsonFeature> {
    match feature_for_entity(data, entity) {
        Some(feature) => {
            let mut features = vec![feature];
            if let Some(point) = point_from_fields(&entity.fields).or_else(|| {
                entity
                    .fields
                    .get("country")
                    .and_then(normalize_country)
                    .as_deref()
                    .and_then(country_centroid)
            }) {
                for (index, polygon) in envelope_polygons(point).into_iter().enumerate() {
                    let mut properties = features[0].properties.clone();
                    properties.insert("geometry_role".into(), "envelope".into());
                    let id = if index == 0 {
                        format!("{}-envelope", entity.id)
                    } else {
                        format!("{}-envelope-{index}", entity.id)
                    };
                    features.push(GeoJsonFeature {
                        feature_type: "Feature",
                        id,
                        geometry: Some(GeoJsonGeometry {
                            geometry_type: "Polygon",
                            coordinates: GeoJsonCoordinates::Polygon(polygon),
                        }),
                        properties,
                    });
                }
            }
            features
        }
        None => Vec::new(),
    }
}

fn feature_for_entity(data: &InvestigationData, entity: &Entity) -> Option<GeoJsonFeature> {
    let country = entity.fields.get("country").and_then(normalize_country);
    let prefix = entity
        .fields
        .get("prefix")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let mut point = point_from_fields(&entity.fields);
    if point.is_none()
        && let Some(code) = country.as_deref()
    {
        point = country_centroid(code);
    }
    if point.is_none() && country.is_none() && prefix.is_none() {
        return None;
    }
    let geometry = point.map(|point| GeoJsonGeometry {
        geometry_type: "Point",
        coordinates: GeoJsonCoordinates::Point(point.coordinates()),
    });
    let mut properties = serde_json::Map::new();
    properties.insert("kind".into(), entity.kind.as_str().into());
    properties.insert("value".into(), entity.value.clone().into());
    properties.insert("label".into(), entity.display_name().into());
    properties.insert("provenance".into(), entity.provenance.as_str().into());
    properties.insert("reference".into(), entity.short_ref().to_string().into());
    properties.insert(
        "canonical_id".into(),
        data.canonical_id(entity.id).to_string().into(),
    );
    if let Some(country) = country {
        properties.insert("country".into(), country.into());
    }
    if let Some(prefix) = prefix {
        properties.insert("prefix".into(), prefix.into());
    }
    if let Some(source) = entity.fields.get("geo_source") {
        properties.insert("geo_source".into(), source.to_string().into());
    }
    Some(GeoJsonFeature {
        feature_type: "Feature",
        id: entity.id.to_string(),
        geometry,
        properties,
    })
}

fn envelope_polygons(point: GeoPoint) -> Vec<Vec<Vec<[f64; 2]>>> {
    let delta: f64 = 0.5;
    let south = (point.latitude - delta).clamp(MIN_LATITUDE, MAX_LATITUDE);
    let north = (point.latitude + delta).clamp(MIN_LATITUDE, MAX_LATITUDE);
    let west = point.longitude - delta;
    let east = point.longitude + delta;
    if west >= MIN_LONGITUDE && east <= MAX_LONGITUDE {
        return vec![closed_ring(west, east, south, north)];
    }
    if west < MIN_LONGITUDE {
        vec![
            closed_ring(wrap_longitude(west), MAX_LONGITUDE, south, north),
            closed_ring(MIN_LONGITUDE, east, south, north),
        ]
    } else {
        vec![
            closed_ring(west, MAX_LONGITUDE, south, north),
            closed_ring(MIN_LONGITUDE, wrap_longitude(east), south, north),
        ]
    }
}

fn wrap_longitude(longitude: f64) -> f64 {
    let wrapped = ((longitude + 180.0).rem_euclid(360.0)) - 180.0;
    if wrapped == MIN_LONGITUDE && longitude > MIN_LONGITUDE {
        MAX_LONGITUDE
    } else {
        wrapped
    }
}

fn closed_ring(west: f64, east: f64, south: f64, north: f64) -> Vec<Vec<[f64; 2]>> {
    vec![vec![
        [west, south],
        [east, south],
        [east, north],
        [west, north],
        [west, south],
    ]]
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GeoJsonFeatureCollection {
    #[serde(rename = "type")]
    pub feature_type: &'static str,
    pub features: Vec<GeoJsonFeature>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GeoJsonFeature {
    #[serde(rename = "type")]
    pub feature_type: &'static str,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geometry: Option<GeoJsonGeometry>,
    pub properties: serde_json::Map<String, serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GeoJsonGeometry {
    #[serde(rename = "type")]
    pub geometry_type: &'static str,
    pub coordinates: GeoJsonCoordinates,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum GeoJsonCoordinates {
    Point([f64; 2]),
    Polygon(Vec<Vec<[f64; 2]>>),
}

pub fn entity_kinds_with_geo(kind: &EntityKind) -> bool {
    matches!(
        kind,
        EntityKind::Ip | EntityKind::Asn | EntityKind::Organization | EntityKind::Location
    )
}

pub fn annotate_geo_fields(fields: &mut Fields, location: &GeoLocation, provenance: Provenance) {
    let _ = provenance;
    if let Some(country) = location.country.as_deref().and_then(normalize_country)
        && fields.get("country").is_none()
    {
        fields.set("country", country);
    }
    if let Some(point) = location.point {
        if fields.get("latitude").and_then(parse_latitude).is_none() {
            fields.set("latitude", format!("{:.6}", point.latitude));
        }
        if fields.get("longitude").and_then(parse_longitude).is_none() {
            fields.set("longitude", format!("{:.6}", point.longitude));
        }
    }
    if fields.get("geo_source").is_none() {
        fields.set("geo_source", location.derivation.as_str());
    }
    if let Some(prefix) = location.prefix.as_deref().filter(|value| !value.is_empty())
        && fields.get("prefix").is_none()
    {
        fields.set("prefix", prefix);
    }
}

/// Best-effort parse of an IP used only for local geo-db matching.
pub fn parse_ip(raw: &str) -> Option<IpAddr> {
    raw.trim().parse().ok()
}

const ALIASES: &[(&str, &str)] = &[
    ("united states", "US"),
    ("usa", "US"),
    ("united kingdom", "GB"),
    ("great britain", "GB"),
    ("australia", "AU"),
];

/// Compact ISO 3166-1 alpha-2 centroids for passive fallback points. Not a boundary file.
const CENTROIDS: &[(&str, f64, f64)] = &[
    ("AD", 42.5, 1.5),
    ("AE", 24.0, 54.0),
    ("AF", 33.0, 65.0),
    ("AL", 41.0, 20.0),
    ("AM", 40.0, 45.0),
    ("AR", -34.0, -64.0),
    ("AT", 47.3, 13.3),
    ("AU", -25.0, 133.0),
    ("AZ", 40.5, 47.5),
    ("BE", 50.8, 4.5),
    ("BG", 43.0, 25.0),
    ("BR", -10.0, -55.0),
    ("CA", 60.0, -95.0),
    ("CH", 47.0, 8.0),
    ("CL", -30.0, -71.0),
    ("CN", 35.0, 105.0),
    ("CO", 4.0, -72.0),
    ("CZ", 49.8, 15.5),
    ("DE", 51.0, 9.0),
    ("DK", 56.0, 10.0),
    ("EG", 27.0, 30.0),
    ("ES", 40.0, -4.0),
    ("FI", 64.0, 26.0),
    ("FR", 46.0, 2.0),
    ("GB", 54.0, -2.0),
    ("GR", 39.0, 22.0),
    ("HK", 22.3, 114.2),
    ("HU", 47.0, 20.0),
    ("ID", -5.0, 120.0),
    ("IE", 53.0, -8.0),
    ("IL", 31.5, 34.8),
    ("IN", 20.0, 77.0),
    ("IR", 32.0, 53.0),
    ("IT", 42.8, 12.8),
    ("JP", 36.0, 138.0),
    ("KE", 1.0, 38.0),
    ("KR", 37.0, 127.5),
    ("MX", 23.0, -102.0),
    ("MY", 2.5, 112.5),
    ("NG", 10.0, 8.0),
    ("NL", 52.3, 5.5),
    ("NO", 62.0, 10.0),
    ("NZ", -41.0, 174.0),
    ("PH", 13.0, 122.0),
    ("PK", 30.0, 70.0),
    ("PL", 52.0, 20.0),
    ("PT", 39.5, -8.0),
    ("RO", 46.0, 25.0),
    ("RU", 60.0, 100.0),
    ("SA", 25.0, 45.0),
    ("SE", 62.0, 15.0),
    ("SG", 1.4, 103.8),
    ("TH", 15.0, 100.0),
    ("TR", 39.0, 35.0),
    ("TW", 23.5, 121.0),
    ("UA", 49.0, 32.0),
    ("US", 38.0, -97.0),
    ("UY", -33.0, -56.0),
    ("VN", 16.0, 108.0),
    ("ZA", -29.0, 24.0),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Entity, EntityKind, Investigation, InvestigationData, Provenance, now};
    use uuid::Uuid;

    #[test]
    fn refuses_nan_inf_and_out_of_range_coordinates() {
        for input in ["NaN", "nan", "inf", "+inf", "-inf", "91", "-90.1", ""] {
            assert!(parse_latitude(input).is_none(), "lat {input:?}");
        }
        for input in ["NaN", "180.1", "-181", "Infinity"] {
            assert!(parse_longitude(input).is_none(), "lon {input:?}");
        }
        assert_eq!(parse_latitude("37.5"), Some(37.5));
        assert_eq!(parse_longitude("-122.4"), Some(-122.4));
        assert!(GeoPoint::new(f64::NAN, 0.0).is_none());
        assert!(GeoPoint::new(0.0, f64::INFINITY).is_none());
    }

    #[test]
    fn nearby_matches_country_or_prefix_and_skips_constituents() {
        let investigation_id = Uuid::new_v4();
        let ip = entity(investigation_id, 1, EntityKind::Ip, "192.0.2.1", |fields| {
            fields.set("country", "US");
            fields.set("prefix", "192.0.2.0/24");
        });
        let other = entity(investigation_id, 2, EntityKind::Ip, "192.0.2.8", |fields| {
            fields.set("country", "US");
        });
        let asn = entity(investigation_id, 3, EntityKind::Asn, "AS64496", |fields| {
            fields.set("prefix", "192.0.2.0/24");
        });
        let foreign = entity(
            investigation_id,
            4,
            EntityKind::Ip,
            "198.51.100.1",
            |fields| {
                fields.set("country", "AU");
            },
        );
        let mut data = InvestigationData::empty(Investigation {
            id: investigation_id,
            name: "geo".into(),
            description: String::new(),
            tags: Vec::new(),
            marking: crate::model::Marking::Unmarked,
            created_at: now(),
            updated_at: now(),
        });
        data.entities = vec![ip.clone(), other.clone(), asn.clone(), foreign];
        let actual = nearby_entities(&data, &ip);
        let values: Vec<&str> = actual.iter().map(|entity| entity.value.as_str()).collect();
        assert!(values.contains(&"192.0.2.8"));
        assert!(values.contains(&"AS64496"));
        assert!(!values.contains(&"198.51.100.1"));
        assert!(!values.contains(&"192.0.2.1"));
    }

    #[test]
    fn geojson_includes_a_point_and_properties() {
        let investigation_id = Uuid::new_v4();
        let mut data = InvestigationData::empty(Investigation {
            id: investigation_id,
            name: "geo".into(),
            description: String::new(),
            tags: Vec::new(),
            marking: crate::model::Marking::Unmarked,
            created_at: now(),
            updated_at: now(),
        });
        data.entities.push(entity(
            investigation_id,
            1,
            EntityKind::Ip,
            "192.0.2.1",
            |fields| {
                fields.set("country", "US");
                fields.set("latitude", "38.0");
                fields.set("longitude", "-97.0");
                fields.set("prefix", "192.0.2.0/24");
                fields.set("geo_source", "collected");
            },
        ));
        let collection = geojson_collection(&data);
        assert_eq!(collection.feature_type, "FeatureCollection");
        assert!(
            collection
                .features
                .iter()
                .any(|feature| feature.geometry.as_ref().is_some_and(|geometry| {
                    geometry.geometry_type == "Point"
                        && matches!(geometry.coordinates, GeoJsonCoordinates::Point(pair) if pair == [-97.0, 38.0])
                })),
            "expected a point {collection:?}"
        );
        assert!(
            collection.features.iter().any(|feature| {
                feature
                    .geometry
                    .as_ref()
                    .is_some_and(|geometry| geometry.geometry_type == "Polygon")
            }),
            "expected a polygon"
        );
        let point = collection
            .features
            .iter()
            .find(|feature| {
                feature
                    .geometry
                    .as_ref()
                    .is_some_and(|geometry| geometry.geometry_type == "Point")
            })
            .expect("point feature");
        assert_eq!(
            point
                .properties
                .get("country")
                .and_then(|value| value.as_str()),
            Some("US")
        );
        assert_eq!(
            point
                .properties
                .get("prefix")
                .and_then(|value| value.as_str()),
            Some("192.0.2.0/24")
        );
        let encoded = serde_json::to_value(&collection).expect("json");
        assert_eq!(encoded["type"], "FeatureCollection");
        assert!(
            encoded["features"]
                .as_array()
                .expect("features")
                .iter()
                .any(|feature| feature["geometry"]["type"] == "Point")
        );
    }

    #[test]
    fn envelope_wraps_the_antimeridian_instead_of_clamping() {
        let point = GeoPoint::new(0.0, 179.8).expect("point");
        let polygons = envelope_polygons(point);
        assert_eq!(
            polygons.len(),
            2,
            "dateline envelope must split, not shrink"
        );
        let lons: Vec<f64> = polygons
            .iter()
            .flat_map(|polygon| polygon.iter().flatten())
            .map(|position| position[0])
            .collect();
        assert!(lons.iter().any(|lon| *lon <= -179.0), "east wrap {lons:?}");
        assert!(lons.iter().any(|lon| *lon >= 179.0), "west piece {lons:?}");
        assert!(
            lons.iter()
                .all(|lon| *lon >= MIN_LONGITUDE && *lon <= MAX_LONGITUDE),
            "wrapped longs stay in range {lons:?}"
        );
        let west = GeoPoint::new(0.0, -179.8).expect("west");
        assert_eq!(envelope_polygons(west).len(), 2);
        let inland = GeoPoint::new(38.0, -97.0).expect("inland");
        assert_eq!(envelope_polygons(inland).len(), 1);
    }

    fn entity(
        investigation_id: Uuid,
        seq: i64,
        kind: EntityKind,
        value: &str,
        annotate: impl FnOnce(&mut Fields),
    ) -> Entity {
        let mut fields = Fields::new();
        annotate(&mut fields);
        Entity {
            id: Uuid::new_v4(),
            investigation_id,
            seq,
            kind,
            value: value.into(),
            label: String::new(),
            notes: String::new(),
            provenance: Provenance::Collected,
            fields,
            source_ids: Vec::new(),
            created_at: now(),
            updated_at: now(),
        }
    }
}
