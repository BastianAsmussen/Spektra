use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use rand::RngExt as _;
use serde::{Deserialize, Serialize};

const IDENTITY_BYTES: usize = 16;
const IDENTITY_MODE: u32 = 0o600;

/// What the agent knows about itself between runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    pub identity: String,
    pub node_id: i64,
    pub credential: String,
}

/// Why the identity could not be read or written.
#[derive(Debug)]
pub enum IdentityError {
    /// The state directory or the file could not be touched.
    Io { path: PathBuf, source: io::Error },
    /// The file exists but does not hold an identity.
    Malformed {
        path: PathBuf,
        source: serde_json::Error,
    },
}

impl std::fmt::Display for IdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::Io {
                ref path,
                ref source,
            } => {
                write!(f, "{}: {source}", path.display())
            }
            Self::Malformed {
                ref path,
                ref source,
            } => write!(
                f,
                "{} is not a readable node identity: {source}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for IdentityError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            Self::Io { ref source, .. } => Some(source),
            Self::Malformed { ref source, .. } => Some(source),
        }
    }
}

/// A fresh identity string: hex over [`IDENTITY_BYTES`] random bytes.
#[must_use]
pub fn generate() -> String {
    let mut bytes = [0_u8; IDENTITY_BYTES];
    rand::rng().fill(&mut bytes[..]);

    hex::encode(bytes)
}

/// Read the stored identity, or `None` if this node has never registered.
///
/// # Errors
///
/// [`IdentityError::Io`] if the file cannot be read, [`IdentityError::Malformed`] if it does not parse.
pub fn load(path: &Path) -> Result<Option<Identity>, IdentityError> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(IdentityError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };

    serde_json::from_str(&raw)
        .map(Some)
        .map_err(|source| IdentityError::Malformed {
            path: path.to_path_buf(),
            source,
        })
}

/// Write the identity, creating the state directory if it is missing.
///
/// # Errors
///
/// [`IdentityError::Io`] or [`IdentityError::Malformed`] if the identity cannot be written.
pub fn store(path: &Path, identity: &Identity) -> Result<(), IdentityError> {
    let io_error = |source: io::Error| IdentityError::Io {
        path: path.to_path_buf(),
        source,
    };

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| IdentityError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let encoded =
        serde_json::to_string_pretty(identity).map_err(|source| IdentityError::Malformed {
            path: path.to_path_buf(),
            source,
        })?;

    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, encoded).map_err(|source| IdentityError::Io {
        path: temporary.clone(),
        source,
    })?;
    fs::set_permissions(&temporary, fs::Permissions::from_mode(IDENTITY_MODE)).map_err(
        |source| IdentityError::Io {
            path: temporary.clone(),
            source,
        },
    )?;
    fs::rename(&temporary, path).map_err(io_error)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("spektra-identity-{name}-{}", generate()));
        fs::create_dir_all(&dir).expect("the temporary directory is creatable");

        dir
    }

    #[test]
    fn an_identity_is_thirty_two_hex_characters() {
        let value = generate();

        assert_eq!(value.len(), IDENTITY_BYTES * 2);
        assert!(value.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn two_identities_do_not_collide() {
        assert_ne!(generate(), generate());
    }

    #[test]
    fn a_missing_file_is_a_node_that_has_never_registered() {
        let dir = temp_dir("missing");

        assert!(
            load(&dir.join("identity.json"))
                .expect("a missing file is not an error")
                .is_none()
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_stored_identity_reads_back() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("nested").join("identity.json");

        let written = Identity {
            identity: generate(),
            node_id: 42,
            credential: "deadbeef".to_owned(),
        };
        store(&path, &written).expect("the identity is writable");

        let read = load(&path)
            .expect("the identity is readable")
            .expect("the identity is present");

        assert_eq!(read.identity, written.identity);
        assert_eq!(read.node_id, 42);
        assert_eq!(read.credential, "deadbeef");

        let mode = fs::metadata(&path)
            .expect("the file exists")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, IDENTITY_MODE);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_file_that_is_not_an_identity_is_rejected() {
        let dir = temp_dir("malformed");
        let path = dir.join("identity.json");
        fs::write(&path, "not json at all").expect("the file is writable");

        assert!(matches!(load(&path), Err(IdentityError::Malformed { .. })));

        fs::remove_dir_all(&dir).ok();
    }
}
