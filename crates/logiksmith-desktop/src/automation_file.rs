pub fn load_automation(path: &Path) -> Result<(AutomationDocument, u16), AutomationFileError> {
    let source = fs::read(path).map_err(|source| AutomationFileError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let text = String::from_utf8_lossy(&source);
    if let Ok(value) = toml::from_str::<toml::Value>(&text) {
        let legacy = ["inputs", "outputs", "knx_bindings", "logic"]
            .into_iter()
            .filter(|field| value.get(*field).is_some())
            .map(|field| FieldError {
                path: (*field).to_owned(),
                message: "legacy top-level field must move inside [[blocks]]".to_owned(),
            })
            .collect::<Vec<_>>();
        if !legacy.is_empty() {
            return Err(AutomationFileError::Invalid(legacy));
        }
    }
    let stored = toml::from_str::<StoredAutomation>(&text).map_err(AutomationFileError::Toml)?;
    build_automation(stored.document.clone()).map_err(AutomationFileError::Invalid)?;
    Ok((stored.document, stored.revision))
}

pub fn serialize_automation(
    document: &AutomationDocument,
    _revision: u16,
) -> Result<Vec<u8>, String> {
    toml::to_string_pretty(&StoredAutomation {
        revision: 0,
        document: document.clone(),
    })
    .map(|text| text.into_bytes())
    .map_err(|error| error.to_string())
}

include!("configuration_loading.rs");
