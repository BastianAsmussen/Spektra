use std::fs;
use std::io;
use std::path::Path;

use prost::Message as _;
use protocol::v1::ChannelPlan;

/// Read the cached plan, or `None` if this node has never been served one.
#[must_use]
pub fn load(path: &Path) -> Option<ChannelPlan> {
    let raw = match fs::read(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return None,
        Err(err) => {
            tracing::warn!(error = %err, path = %path.display(), "could not read the cached plan");

            return None;
        }
    };

    match ChannelPlan::decode(raw.as_slice()) {
        Ok(plan) => Some(plan),
        Err(err) => {
            tracing::warn!(error = %err, "the cached channel plan does not decode, discarding it");

            None
        }
    }
}

/// Cache a served plan.
///
/// # Errors
///
/// [`io::Error`] if the state directory or the file cannot be written.
pub fn store(path: &Path, plan: &ChannelPlan) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut encoded = Vec::with_capacity(plan.encoded_len());
    plan.encode(&mut encoded)
        .map_err(|err| io::Error::other(err.to_string()))?;

    let temporary = path.with_extension("pb.tmp");
    fs::write(&temporary, encoded)?;
    fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use protocol::v1::{ChannelAssignment, Modulation};

    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "spektra-plan-{name}-{}",
            crate::identity::generate()
        ));
        fs::create_dir_all(&dir).expect("the temporary directory is creatable");

        dir
    }

    fn plan() -> ChannelPlan {
        ChannelPlan {
            protocol_version: protocol::PROTOCOL_VERSION.to_owned(),
            plan_version: 7,
            channels: vec![ChannelAssignment {
                frequency_hz: 89_700_000,
                modulation: i32::from(Modulation::Fm),
                label: "DR P4 Nordjylland".to_owned(),
                bandwidth_hz: 0,
            }],
            issued_at: None,
        }
    }

    #[test]
    fn a_node_that_has_never_been_served_has_no_plan() {
        let dir = scratch("absent");

        assert!(load(&dir.join("plan.pb")).is_none());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_cached_plan_reads_back() {
        let dir = scratch("roundtrip");
        let path = dir.join("nested").join("plan.pb");

        store(&path, &plan()).expect("the plan is writable");
        let read = load(&path).expect("the plan is present");

        assert_eq!(read.plan_version, 7);
        assert_eq!(read.channels.len(), 1);
        assert_eq!(
            read.channels.first().map(|channel| channel.frequency_hz),
            Some(89_700_000)
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_cache_that_does_not_decode_is_discarded() {
        let dir = scratch("corrupt");
        let path = dir.join("plan.pb");
        fs::write(&path, [0xFF_u8; 32]).expect("writable");

        assert!(load(&path).is_none());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_later_plan_replaces_an_earlier_one() {
        let dir = scratch("replace");
        let path = dir.join("plan.pb");

        store(&path, &plan()).expect("writable");
        store(
            &path,
            &ChannelPlan {
                plan_version: 9,
                channels: Vec::new(),
                ..plan()
            },
        )
        .expect("writable");

        let read = load(&path).expect("present");
        assert_eq!(read.plan_version, 9);
        assert!(read.channels.is_empty());

        fs::remove_dir_all(&dir).ok();
    }
}
