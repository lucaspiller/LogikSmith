use logiksmith_core::{BlockId, PersistentState, StateValue};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

const VERSION: u64 = 1;
const TEMP_FILE_ATTEMPTS: usize = 32;

#[derive(Clone, Debug)]
pub(crate) struct PersistentStateStore {
    path: PathBuf,
}

#[derive(Debug)]
pub(crate) struct LoadedPersistentState {
    pub(crate) states: BTreeMap<BlockId, PersistentState>,
    pub(crate) pruned: bool,
}

#[derive(Debug, Deserialize)]
struct StoredSnapshot {
    version: u64,
    blocks: BTreeMap<String, BTreeMap<String, StoredValue>>,
}

#[derive(Debug, Deserialize)]
struct StoredValue {
    kind: String,
    value: JsonValue,
}

#[derive(Debug, Serialize)]
struct SnapshotToStore {
    version: u64,
    blocks: BTreeMap<String, BTreeMap<String, StoredValueToStore>>,
}

#[derive(Debug, Serialize)]
struct StoredValueToStore {
    kind: &'static str,
    value: JsonValue,
}

impl PersistentStateStore {
    pub(crate) fn new(automation_path: &Path) -> Self {
        let mut path = automation_path.as_os_str().to_os_string();
        path.push(".persistent.json");
        Self {
            path: PathBuf::from(path),
        }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn load(
        &self,
        active_block_ids: &[BlockId],
    ) -> Result<LoadedPersistentState, String> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(LoadedPersistentState {
                    states: BTreeMap::new(),
                    pruned: false,
                });
            }
            Err(error) => return Err(self.io_error(error)),
        };

        let snapshot: StoredSnapshot = serde_json::from_slice(&bytes).map_err(|error| {
            self.invalid(format!("invalid JSON ({error})"))
        })?;
        if snapshot.version != VERSION {
            return Err(self.invalid(format!("unsupported version {}", snapshot.version)));
        }

        let mut states = BTreeMap::new();
        let mut pruned = false;
        for (block_name, values) in snapshot.blocks {
            let Some(block_id) = active_block_ids
                .iter()
                .find(|block_id| block_id.as_str() == block_name)
            else {
                pruned = true;
                continue;
            };
            let mut state = BTreeMap::new();
            for (key, value) in values {
                let value = state_value_from_stored(&value).map_err(|message| {
                    self.invalid(format!("block {block_name:?}, key {key:?}: {message}"))
                })?;
                state.insert(key, value);
            }
            states.insert(block_id.clone(), state);
        }

        Ok(LoadedPersistentState { states, pruned })
    }

    pub(crate) fn save(
        &self,
        states: &BTreeMap<BlockId, PersistentState>,
    ) -> Result<(), String> {
        let mut blocks = BTreeMap::new();
        for (block_id, state) in states.iter().filter(|(_, state)| !state.is_empty()) {
            let mut values = BTreeMap::new();
            for (key, value) in state {
                let stored = stored_value(value).map_err(|message| {
                    self.invalid(format!("block {block_id:?}, key {key:?}: {message}"))
                })?;
                values.insert(key.clone(), stored);
            }
            blocks.insert(block_id.to_string(), values);
        }
        let snapshot = SnapshotToStore {
            version: VERSION,
            blocks,
        };
        let bytes = serde_json::to_vec_pretty(&snapshot)
            .map_err(|error| self.invalid(format!("could not serialize snapshot ({error})")))?;

        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        let mut temporary = None;
        for attempt in 0..TEMP_FILE_ATTEMPTS {
            let candidate = parent.join(self.temporary_name(attempt));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&candidate)
            {
                Ok(file) => {
                    temporary = Some((candidate, file));
                    break;
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(self.io_error(error)),
            }
        }
        let Some((temporary, mut file)) = temporary else {
            return Err(self.invalid("could not allocate a unique temporary file"));
        };

        let result = (|| -> io::Result<()> {
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, &self.path)
        })();
        if let Err(error) = result {
            let _ = fs::remove_file(&temporary);
            return Err(self.io_error(error));
        }
        Ok(())
    }

    fn temporary_name(&self, attempt: usize) -> OsString {
        let mut name = self
            .path
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("persistent.json"))
            .to_os_string();
        name.push(format!(".tmp-{}-{attempt}", std::process::id()));
        name
    }

    fn invalid(&self, message: impl Into<String>) -> String {
        format!(
            "persistent state sidecar {}: {}",
            self.path.display(),
            message.into()
        )
    }

    fn io_error(&self, source: io::Error) -> String {
        self.invalid(source.to_string())
    }
}

fn state_value_from_stored(value: &StoredValue) -> Result<StateValue, String> {
    match value.kind.as_str() {
        "bool" => value
            .value
            .as_bool()
            .map(StateValue::Bool)
            .ok_or_else(|| "bool value expected".to_owned()),
        "integer" => value
            .value
            .as_i64()
            .map(StateValue::Integer)
            .ok_or_else(|| "i64 integer value expected".to_owned()),
        "number" => value
            .value
            .as_f64()
            .filter(|value| value.is_finite())
            .map(StateValue::Number)
            .ok_or_else(|| "finite number value expected".to_owned()),
        "string" => value
            .value
            .as_str()
            .map(|value| StateValue::String(value.to_owned()))
            .ok_or_else(|| "string value expected".to_owned()),
        kind => Err(format!("unsupported kind {kind:?}")),
    }
}

fn stored_value(value: &StateValue) -> Result<StoredValueToStore, &'static str> {
    match value {
        StateValue::Bool(value) => Ok(StoredValueToStore {
            kind: "bool",
            value: JsonValue::Bool(*value),
        }),
        StateValue::Integer(value) => Ok(StoredValueToStore {
            kind: "integer",
            value: JsonValue::Number((*value).into()),
        }),
        StateValue::Number(value) => Ok(StoredValueToStore {
            kind: "number",
            value: serde_json::Number::from_f64(*value)
                .map(JsonValue::Number)
                .ok_or("finite number value expected")?,
        }),
        StateValue::String(value) => Ok(StoredValueToStore {
            kind: "string",
            value: JsonValue::String(value.clone()),
        }),
    }
}
