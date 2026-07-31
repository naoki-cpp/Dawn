use super::CatalogError;
use dawn_core::fitting::ModuleDefinition;
use dawn_core::ship_type::ShipTypeDefinition;
use std::collections::HashSet;
use std::path::Path;

pub(super) fn validate_required_ids(
    modules: &[ModuleDefinition],
    ship_types: &[ShipTypeDefinition],
    modules_path: &Path,
    ship_types_path: &Path,
) -> Result<(), CatalogError> {
    let module_ids: HashSet<_> = modules.iter().map(|definition| definition.id).collect();
    for required in crate::modules::REQUIRED_MODULE_IDS {
        if !module_ids.contains(required) {
            return validation_error(
                "module",
                modules_path,
                format!("required module id {} is missing", required.0),
            );
        }
    }

    let ship_type_ids: HashSet<_> = ship_types.iter().map(|definition| definition.id).collect();
    for required in crate::ship_types::REQUIRED_SHIP_TYPE_IDS {
        if !ship_type_ids.contains(required) {
            return validation_error(
                "ship-type",
                ship_types_path,
                format!("required ship type id {} is missing", required.0),
            );
        }
    }
    Ok(())
}

pub(super) fn validate_positive_f64(
    category: &'static str,
    path: &Path,
    id: u32,
    field: &str,
    value: f64,
) -> Result<(), CatalogError> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else {
        validation_error(
            category,
            path,
            format!("id {id} has invalid {field} value {value}"),
        )
    }
}

pub(super) fn validate_positive_f32(
    category: &'static str,
    path: &Path,
    id: u32,
    field: &str,
    value: f32,
) -> Result<(), CatalogError> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else {
        validation_error(
            category,
            path,
            format!("id {id} has invalid {field} value {value}"),
        )
    }
}

pub(super) fn validate_non_negative_f64(
    category: &'static str,
    path: &Path,
    id: u32,
    field: &str,
    value: f64,
) -> Result<(), CatalogError> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        validation_error(
            category,
            path,
            format!("id {id} has invalid {field} value {value}"),
        )
    }
}

pub(super) fn validate_non_negative_f32(
    category: &'static str,
    path: &Path,
    id: u32,
    field: &str,
    value: f32,
) -> Result<(), CatalogError> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        validation_error(
            category,
            path,
            format!("id {id} has invalid {field} value {value}"),
        )
    }
}

pub(super) fn validation_error<T>(
    category: &'static str,
    path: &Path,
    message: impl Into<String>,
) -> Result<T, CatalogError> {
    Err(CatalogError::Validation {
        category,
        path: path.to_path_buf(),
        message: message.into(),
    })
}
