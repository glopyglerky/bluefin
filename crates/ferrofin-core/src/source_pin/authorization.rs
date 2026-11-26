//! Retain the input's authorization classification, without granting access.
//! Existing repository/visibility services remain the policy authority.
use super::{BaseItemEntity, HashMap, OpenedSourceGeneration, PinnedInput, ServiceError, Uuid};
use ferrofin_db::entities::users::UserEntity;
use ferrofin_model::data::BaseItemKind;
use ferrofin_traits::persistence::ItemRepository;

pub(super) struct LibraryAnchor {
    row: BaseItemEntity,
    origins: Vec<String>,
}

fn references(row: &BaseItemEntity) -> [Option<&str>; 6] {
    [
        row.parent_id.as_deref(),
        row.owner_id.as_deref(),
        row.channel_id.as_deref(),
        row.season_id.as_deref(),
        row.series_id.as_deref(),
        row.top_parent_id.as_deref(),
    ]
}

fn covers(library: &BaseItemEntity, path: &str) -> bool {
    library
        .path
        .iter()
        .cloned()
        .chain(crate::item_data::stored_physical_locations(
            library.data.as_deref(),
        ))
        .any(|root| {
            std::path::Path::new(&root).is_absolute()
                && std::path::Path::new(path).starts_with(root)
        })
}

pub(super) fn anchors(
    captured: &OpenedSourceGeneration,
) -> Result<Vec<LibraryAnchor>, ServiceError> {
    for row in &captured.ancestry {
        for reference in references(row).into_iter().flatten() {
            let id = Uuid::parse_str(reference).map_err(|_| {
                ServiceError::unavailable("source authorization reference is invalid")
            })?;
            if !id.is_nil()
                && !captured
                    .ancestry
                    .iter()
                    .any(|row| Uuid::parse_str(&row.id).ok() == Some(id))
            {
                return Err(ServiceError::unavailable(
                    "source authorization ancestry is incomplete",
                ));
            }
        }
    }
    // These are conservative origin anchors, never authorization grants. The
    // existing visible-library/parental services must approve every one later.
    let anchors: Vec<_> = captured
        .libraries
        .iter()
        .filter_map(|row| {
            let logical = captured
                .ancestry
                .iter()
                .any(|ancestor| ancestor.id == row.id);
            let origins: Vec<String> = [&captured.item, &captured.source]
                .into_iter()
                .filter_map(|source| source.path.as_deref())
                .filter(|path| covers(row, path))
                .map(ToOwned::to_owned)
                .collect();
            (logical || !origins.is_empty()).then(|| LibraryAnchor {
                row: row.clone(),
                origins,
            })
        })
        .collect();
    if anchors.is_empty() {
        return Err(ServiceError::unavailable(
            "source input has no provable library origin",
        ));
    }
    Ok(anchors)
}

impl PinnedInput {
    pub(crate) async fn authorize_classification(
        &self,
        items: &dyn ItemRepository,
        visibility: &crate::item_visibility::ItemVisibility,
        user: &UserEntity,
    ) -> Result<(), ServiceError> {
        let mut originals: HashMap<Uuid, &BaseItemEntity> = self
            .captured
            .ancestry
            .iter()
            .chain(self.anchors.iter().map(|a| &a.row))
            .map(|row| Uuid::parse_str(&row.id).map(|id| (id, row)))
            .collect::<Result<_, _>>()
            .map_err(|_| ServiceError::unavailable("source authorization identity is invalid"))?;
        let ids: Vec<_> = originals.keys().copied().collect();
        let current: HashMap<_, _> = items
            .retrieve_items(&ids)
            .await?
            .into_iter()
            .filter_map(|row| Uuid::parse_str(&row.id).ok().map(|id| (id, row)))
            .collect();
        if current.len() != originals.len() {
            return Err(ServiceError::not_found(
                "source authorization anchor was removed",
            ));
        }
        for (&id, original) in &originals {
            let now = &current[&id];
            if references(original) != references(now) {
                return Err(ServiceError::Forbidden(
                    "source authorization ancestry changed".into(),
                ));
            }
        }
        let allowed = items.visible_library_ids(user).await?;
        for anchor in &self.anchors {
            let id = Uuid::parse_str(&anchor.row.id)
                .map_err(|_| ServiceError::unavailable("source library identity is invalid"))?;
            if !allowed.contains(&id)
                || anchor
                    .origins
                    .iter()
                    .any(|path| !covers(&current[&id], path))
            {
                return Err(ServiceError::Forbidden(
                    "source input library is no longer authorized".into(),
                ));
            }
        }
        for original in [&self.captured.item, &self.captured.source] {
            let id = Uuid::parse_str(&original.id)
                .map_err(|_| ServiceError::unavailable("source pin identity is invalid"))?;
            for row in [&current[&id], original] {
                if visibility
                    .visible(std::slice::from_ref(row), user, true)
                    .await?
                    != [true]
                {
                    return Err(ServiceError::Forbidden(
                        "source pin input is no longer visible".into(),
                    ));
                }
            }
        }
        // Original ancestors' tag/rating classification must survive a move or
        // metadata rewrite too. Skip only allowed-tag requirements on parents,
        // as the existing standalone visibility walk does.
        originals.retain(|_, row| {
            BaseItemKind::from_stored_type_name(&row.type_) != Some(BaseItemKind::UserRootFolder)
        });
        let frozen: Vec<_> = originals.into_values().cloned().collect();
        let fresh: Vec<_> = current.into_values().collect();
        for rows in [&frozen, &fresh] {
            if visibility
                .parentally_allowed(rows, user, true)
                .await?
                .iter()
                .any(|allowed| !allowed)
            {
                return Err(ServiceError::Forbidden(
                    "source input classification is no longer authorized".into(),
                ));
            }
        }
        Ok(())
    }
}
