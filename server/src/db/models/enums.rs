use diesel_derive_enum::DbEnum;
use serde::{Deserialize, Serialize};
use strum::{Display, EnumString, IntoStaticStr};
use utoipa::ToSchema;

/// Modulation scheme of a monitored channel.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    DbEnum,
    Serialize,
    Deserialize,
    ToSchema,
    Display,
    IntoStaticStr,
    EnumString,
)]
#[db_enum(existing_type_path = "crate::db::schema::sql_types::Modulation")]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum Modulation {
    /// Analog FM broadcast.
    Fm,
    /// Digital Audio Broadcasting, Band III.
    Dab,
}

/// Where an alarm sits in its lifecycle.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    DbEnum,
    Serialize,
    Deserialize,
    ToSchema,
    Display,
    IntoStaticStr,
    EnumString,
)]
#[db_enum(existing_type_path = "crate::db::schema::sql_types::AlarmState")]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum AlarmState {
    /// Raised by the detector, nobody has looked at it.
    Open,
    /// An operator has seen it and taken ownership.
    Acknowledged,
    /// A technician is checking the station.
    UnderVerification,
    /// Resolved, with the field result recorded.
    Closed,
}

/// A derived signal-quality dimension.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    DbEnum,
    Serialize,
    Deserialize,
    ToSchema,
    Display,
    IntoStaticStr,
    EnumString,
)]
#[db_enum(existing_type_path = "crate::db::schema::sql_types::Metric")]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum Metric {
    /// Signal strength in dBFS.
    SignalStrength,
    /// Signal-to-noise ratio in dB.
    SignalToNoise,
    /// Observed minus expected carrier frequency, in Hz.
    CarrierOffset,
    /// Share of demodulated blocks that failed, 0.0 to 1.0.
    DemodErrorRate,
    /// Share of the channel bandwidth carrying energy, 0.0 to 1.0.
    SpectrumOccupancy,
}

/// How wide a rollup bucket is.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    DbEnum,
    Serialize,
    Deserialize,
    ToSchema,
    Display,
    IntoStaticStr,
    EnumString,
)]
#[db_enum(existing_type_path = "crate::db::schema::sql_types::RollupResolution")]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum RollupResolution {
    /// One hour of windows.
    Hourly,
    /// One day of windows.
    Daily,
    /// Seven days of windows, starting Monday.
    Weekly,
}

/// Where a dispatched work order has got to.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    DbEnum,
    Serialize,
    Deserialize,
    ToSchema,
    Display,
    IntoStaticStr,
    EnumString,
)]
#[db_enum(existing_type_path = "crate::db::schema::sql_types::WorkOrderStatus")]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum WorkOrderStatus {
    /// Sent to a technician, not yet reported back.
    Assigned,
    /// The technician has been to the station and recorded what they found.
    Completed,
}

/// Give each enum the borrowed form of its own label.
macro_rules! label {
    ($($name:ident),+ $(,)?) => {
        $(
            impl $name {
                /// The `PostgreSQL` label for this variant.
                #[must_use]
                pub fn label(self) -> &'static str {
                    self.into()
                }
            }
        )+
    };
}

label!(
    Modulation,
    AlarmState,
    Metric,
    RollupResolution,
    WorkOrderStatus
);
