use std::fs;
use std::path::Path;

const UPTIME_PATH: &str = "/proc/uptime";
const LOADAVG_PATH: &str = "/proc/loadavg";
const THERMAL_ZONES: &str = "/sys/class/thermal";
const MILLIDEGREES: f64 = 1_000.0;

/// One reading of the node's own operational state.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Health {
    pub uptime_seconds: Option<f64>,
    pub load_1m: Option<f64>,
    pub load_5m: Option<f64>,
    pub load_15m: Option<f64>,
    pub cpu_temperature_celsius: Option<f64>,
}

impl Health {
    /// Read the current state from the running kernel. A reading the kernel does not offer is `None`.
    #[must_use]
    pub fn read() -> Self {
        let load = load_average(Path::new(LOADAVG_PATH));

        Self {
            uptime_seconds: uptime(Path::new(UPTIME_PATH)),
            load_1m: load.map(|[one, _, _]| one),
            load_5m: load.map(|[_, five, _]| five),
            load_15m: load.map(|[_, _, fifteen]| fifteen),
            cpu_temperature_celsius: temperature(Path::new(THERMAL_ZONES)),
        }
    }
}

fn uptime(path: &Path) -> Option<f64> {
    fs::read_to_string(path)
        .ok()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

fn load_average(path: &Path) -> Option<[f64; 3]> {
    let raw = fs::read_to_string(path).ok()?;
    let mut fields = raw.split_whitespace();

    Some([
        fields.next()?.parse().ok()?,
        fields.next()?.parse().ok()?,
        fields.next()?.parse().ok()?,
    ])
}

fn temperature(root: &Path) -> Option<f64> {
    let zones = fs::read_dir(root).ok()?;

    zones
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("thermal_zone"))
        })
        .filter_map(|entry| {
            let raw = fs::read_to_string(entry.path().join("temp")).ok()?;
            let millidegrees: f64 = raw.trim().parse().ok()?;

            Some(millidegrees / MILLIDEGREES)
        })
        .filter(|celsius| (-40.0..=150.0).contains(celsius))
        .fold(None, |warmest: Option<f64>, celsius| {
            Some(warmest.map_or(celsius, |current| current.max(celsius)))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "spektra-health-{name}-{}",
            crate::identity::generate()
        ));
        fs::create_dir_all(&dir).expect("the temporary directory is creatable");

        dir
    }

    #[test]
    fn reads_uptime_from_the_first_field() {
        let dir = scratch("uptime");
        let path = dir.join("uptime");
        fs::write(&path, "12345.67 98765.43\n").expect("writable");

        assert!(uptime(&path).is_some_and(|value| (value - 12_345.67).abs() < f64::EPSILON));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reads_the_three_load_averages() {
        let dir = scratch("loadavg");
        let path = dir.join("loadavg");
        fs::write(&path, "0.42 0.31 0.20 1/234 5678\n").expect("writable");

        let [one, five, fifteen] = load_average(&path).expect("the file parses");

        assert!((one - 0.42).abs() < f64::EPSILON);
        assert!((five - 0.31).abs() < f64::EPSILON);
        assert!((fifteen - 0.20).abs() < f64::EPSILON);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn takes_the_warmest_thermal_zone() {
        let dir = scratch("thermal");
        for (zone, millidegrees) in [("thermal_zone0", "41200"), ("thermal_zone1", "57800")] {
            let path = dir.join(zone);
            fs::create_dir_all(&path).expect("creatable");
            fs::write(path.join("temp"), millidegrees).expect("writable");
        }

        assert!(temperature(&dir).is_some_and(|value| (value - 57.8).abs() < f64::EPSILON));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ignores_a_zone_reporting_an_impossible_temperature() {
        let dir = scratch("implausible");
        for (zone, millidegrees) in [("thermal_zone0", "44000"), ("thermal_zone1", "-274000")] {
            let path = dir.join(zone);
            fs::create_dir_all(&path).expect("creatable");
            fs::write(path.join("temp"), millidegrees).expect("writable");
        }

        assert!(temperature(&dir).is_some_and(|value| (value - 44.0).abs() < f64::EPSILON));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        let dir = scratch("absent");

        assert_eq!(uptime(&dir.join("nope")), None);
        assert_eq!(load_average(&dir.join("nope")), None);
        assert_eq!(temperature(&dir.join("nope")), None);

        fs::remove_dir_all(&dir).ok();
    }
}
