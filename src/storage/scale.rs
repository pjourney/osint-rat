//! Deterministic graph seeding used by scale benchmarks (not a collector).

use color_eyre::eyre::Result;
use uuid::Uuid;

use super::graph::{catalogue_link_type, upsert_entity, upsert_relationship};
use super::{Store, touch_investigation};
use crate::model::{Confidence, EntityKey, EntityKind, Provenance, relation};

impl Store {
    /// Inserts `entity_count` domain entities and a sparse relationship chain.
    ///
    /// Used by `os-int bench` and scale tests. Not an investigation mutation an analyst
    /// would run; callers still go through `assert_can_mutate`.
    pub fn seed_scale_graph(&mut self, investigation_id: Uuid, entity_count: usize) -> Result<()> {
        self.assert_can_mutate(investigation_id)?;
        if entity_count == 0 {
            return Ok(());
        }
        let transaction = self.connection_mut().transaction()?;
        let mut previous: Option<Uuid> = None;
        for index in 0..entity_count {
            let key = EntityKey::new(EntityKind::Domain, &format!("n{index}.scale.test"));
            let (entity_id, _) = upsert_entity(
                &transaction,
                investigation_id,
                &key,
                Provenance::Analyst,
                None,
            )?;
            if let Some(from_id) = previous
                && index % 3 == 0
            {
                let _ = upsert_relationship(
                    &transaction,
                    investigation_id,
                    from_id,
                    relation::ASSOCIATED_WITH,
                    entity_id,
                    Provenance::Analyst,
                    Confidence::Low,
                )?;
                catalogue_link_type(&transaction, investigation_id, relation::ASSOCIATED_WITH)?;
            }
            previous = Some(entity_id);
        }
        touch_investigation(&transaction, investigation_id)?;
        transaction.commit()?;
        Ok(())
    }
}
