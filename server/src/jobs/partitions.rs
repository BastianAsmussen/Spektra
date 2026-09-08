use chrono::{Datelike as _, NaiveDate, Utc};
use diesel::connection::SimpleConnection as _;
use diesel::pg::PgConnection;
use diesel::prelude::*;
use diesel::sql_types::Text;

const PARENT: &str = "measurements";

const DEFAULT_PARTITION: &str = "measurements_default";

#[derive(QueryableByName)]
struct PartitionRow {
    #[diesel(sql_type = Text)]
    relname: String,
}

/// The day after `start`.
#[must_use]
pub const fn next_day(start: NaiveDate) -> Option<NaiveDate> {
    start.succ_opt()
}

/// What a day's partition is called.
#[must_use]
pub fn partition_name(start: NaiveDate) -> String {
    format!(
        "{PARENT}_{:04}_{:02}_{:02}",
        start.year(),
        start.month(),
        start.day()
    )
}

/// Every partition currently attached to `measurements`, default included.
///
/// # Errors
///
/// Returns the diesel error if the catalog query fails.
pub fn existing(conn: &mut PgConnection) -> QueryResult<Vec<String>> {
    let rows: Vec<PartitionRow> = diesel::sql_query(
        "SELECT child.relname \
         FROM pg_inherits \
         JOIN pg_class child ON child.oid = pg_inherits.inhrelid \
         JOIN pg_class parent ON parent.oid = pg_inherits.inhparent \
         WHERE parent.relname = 'measurements' \
         ORDER BY child.relname",
    )
    .load(conn)?;

    Ok(rows.into_iter().map(|row| row.relname).collect())
}

fn ensure_day(conn: &mut PgConnection, start: NaiveDate, known: &[String]) -> QueryResult<bool> {
    let Some(end) = next_day(start) else {
        return Ok(false);
    };
    let name = partition_name(start);

    if known.contains(&name) {
        return Ok(false);
    }

    conn.transaction(|conn| {
        conn.batch_execute(&format!(
            "ALTER TABLE {PARENT} DETACH PARTITION {DEFAULT_PARTITION};\n\
             CREATE TABLE {name} PARTITION OF {PARENT} \
                FOR VALUES FROM ('{start}') TO ('{end}');\n\
             WITH moved AS (\n\
                 DELETE FROM {DEFAULT_PARTITION}\n\
                 WHERE window_start >= '{start}' AND window_start < '{end}'\n\
                 RETURNING *\n\
             )\n\
             INSERT INTO {PARENT} (\n\
                 id, node_id, channel_id, metric, window_start, window_end,\n\
                 min, max, mean, median, stddev, p95, sample_count, created_at\n\
             )\n\
             SELECT id, node_id, channel_id, metric, window_start, window_end,\n\
                    min, max, mean, median, stddev, p95, sample_count, created_at\n\
             FROM moved;\n\
             ALTER TABLE {PARENT} ATTACH PARTITION {DEFAULT_PARTITION} DEFAULT;"
        ))
    })?;

    Ok(true)
}

/// Create every partition from `first` to `last`, inclusive.
///
/// # Errors
///
/// Returns the diesel error from the first day that fails.
pub fn ensure_range(
    conn: &mut PgConnection,
    first: NaiveDate,
    last: NaiveDate,
) -> QueryResult<Vec<String>> {
    let mut known = existing(conn)?;
    let mut start = first;
    let mut created = Vec::new();

    while start <= last {
        if ensure_day(conn, start, &known)? {
            let name = partition_name(start);
            known.push(name.clone());
            created.push(name);
        }
        let Some(next) = next_day(start) else {
            break;
        };
        start = next;
    }

    Ok(created)
}

/// Drop every daily partition that ends at or before `cutoff`.
///
/// # Errors
///
/// Returns the diesel error if the catalog query or a drop fails.
pub fn drop_before(conn: &mut PgConnection, cutoff: NaiveDate) -> QueryResult<Vec<String>> {
    let mut dropped = Vec::new();

    for name in existing(conn)? {
        if name == DEFAULT_PARTITION {
            continue;
        }

        let Some(start) = parse_partition_name(&name) else {
            continue;
        };

        let Some(end) = next_day(start) else {
            continue;
        };

        if end > cutoff {
            continue;
        }

        conn.batch_execute(&format!("DROP TABLE {name};"))?;
        dropped.push(name);
    }

    Ok(dropped)
}

fn parse_partition_name(name: &str) -> Option<NaiveDate> {
    let suffix = name.strip_prefix(PARENT)?.strip_prefix('_')?;
    let (year, rest) = suffix.split_once('_')?;
    let (month, day) = rest.split_once('_')?;

    NaiveDate::from_ymd_opt(year.parse().ok()?, month.parse().ok()?, day.parse().ok()?)
}

/// Today, by the server's clock.
#[must_use]
pub fn today() -> NaiveDate {
    Utc::now().date_naive()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_partition_is_named_after_its_day() {
        let start = NaiveDate::from_ymd_opt(2026, 9, 17).expect("a real date");

        assert_eq!(partition_name(start), "measurements_2026_09_17");
    }

    #[test]
    fn a_partition_name_round_trips() {
        for (year, month, day) in [(2026, 1, 1), (2026, 9, 17), (2027, 12, 31)] {
            let start = NaiveDate::from_ymd_opt(year, month, day).expect("a real date");

            assert_eq!(parse_partition_name(&partition_name(start)), Some(start));
        }
    }

    #[test]
    fn the_default_partition_is_not_one_of_ours() {
        assert_eq!(parse_partition_name(DEFAULT_PARTITION), None);
        assert_eq!(parse_partition_name("measurements"), None);
        assert_eq!(parse_partition_name("measurements_2026_09"), None);
        assert_eq!(parse_partition_name("rollups_2026_09_17"), None);
    }

    #[test]
    fn the_last_day_of_a_month_rolls_into_the_next() {
        let september = NaiveDate::from_ymd_opt(2026, 9, 30).expect("a real date");

        assert_eq!(next_day(september), NaiveDate::from_ymd_opt(2026, 10, 1));
    }

    #[test]
    fn december_rolls_into_january() {
        let december = NaiveDate::from_ymd_opt(2026, 12, 31).expect("a real date");

        assert_eq!(next_day(december), NaiveDate::from_ymd_opt(2027, 1, 1));
    }
}
