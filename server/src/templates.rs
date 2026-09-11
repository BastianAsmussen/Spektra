use askama::Template;
use chrono::NaiveDateTime;
use serde::Serialize;

const ZONE: chrono_tz::Tz = chrono_tz::Europe::Copenhagen;

#[derive(Debug, Clone, Default, Serialize)]
pub struct Stamp {
    pub machine: String,
    pub text: String,
}

impl Stamp {
    /// Whether there is a time to show at all.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

/// Render one stored instant for display.
#[must_use]
pub fn stamp(at: NaiveDateTime) -> Stamp {
    let local = at.and_utc().with_timezone(&ZONE);

    Stamp {
        machine: local.to_rfc3339(),
        text: local.format("%Y-%m-%d %H:%M").to_string(),
    }
}

/// The same, for a column that may be absent.
#[must_use]
pub fn stamp_or_empty(at: Option<NaiveDateTime>) -> Stamp {
    at.map(stamp).unwrap_or_default()
}

#[derive(Debug, Serialize)]
pub struct NodeTile {
    pub id: i64,
    pub name: String,
    pub state: &'static str,
    pub at: Stamp,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub open_alarms: i64,
}

#[derive(Template)]
#[template(path = "fragments/fleet.html")]
pub struct FleetPage {
    pub nodes: Vec<NodeTile>,
    pub next: String,
    pub first: bool,
}

#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexTemplate {
    pub nodes: Vec<NodeTile>,
    pub shown: usize,
    pub next: String,
    pub panel: String,
    pub nodes_total: usize,
    pub silent: usize,
    pub open_alarms: i64,
    pub open_orders: i64,
    pub user_name: String,
    pub user_role: String,
    pub live: bool,
}

#[derive(Debug, Default)]
pub struct Chrome {
    pub nodes_total: usize,
    pub silent: usize,
    pub open_alarms: i64,
    pub open_orders: i64,
    pub user_name: String,
    pub user_role: String,
}

#[derive(Template)]
#[template(path = "fragments/node_status.html")]
pub struct NodeStatusFragment {
    pub node_id: i64,
    pub state: &'static str,
    pub at: Stamp,
}

/// The login form.
#[derive(Template)]
#[template(path = "login.html")]
pub struct LoginTemplate {
    pub error: Option<String>,
}

pub struct AdminUserRow {
    pub id: i64,
    pub email: String,
    pub full_name: String,
    pub role: String,
    pub deactivated: bool,
}

/// One node as the administration page lists it.
pub struct AdminNodeRow {
    pub id: i64,
    pub name: String,
    pub suspended: bool,
    pub enrolled: bool,
}

/// Somebody a node can be handed to.
pub struct Owner {
    pub id: i64,
    pub name: String,
    pub selected: bool,
}

/// The node edit form, opened from the pencil.
#[derive(Template)]
#[template(path = "fragments/node_edit.html")]
pub struct NodeEditForm {
    pub id: i64,
    pub name: String,
    pub latitude: String,
    pub longitude: String,
    pub owner_id: Option<i64>,
    pub owners: Vec<Owner>,
    pub may_assign_owner: bool,
    pub report_interval_seconds: i32,
    pub interval_min: i32,
    pub interval_max: i32,
}

/// A credential, rendered once and never again.
#[derive(Template)]
#[template(path = "fragments/credential.html")]
pub struct CredentialReveal {
    pub node_id: i64,
    pub node_name: String,
    pub credential: String,
}

/// The accounts, either the whole list or one row swapped in place.
#[derive(Template)]
#[template(path = "fragments/admin_users.html")]
pub struct AdminUsersFragment {
    pub users: Vec<AdminUserRow>,
    pub roles: Vec<(&'static str, &'static str)>,
    pub current_user_id: i64,
    pub list: bool,
}

/// The nodes, either the whole list or one row swapped in place.
#[derive(Template)]
#[template(path = "fragments/admin_nodes.html")]
pub struct AdminNodesFragment {
    pub nodes: Vec<AdminNodeRow>,
    pub list: bool,
}

/// The administration page.
#[derive(Template)]
#[template(path = "admin.html")]
pub struct AdminTemplate {
    pub users: Vec<AdminUserRow>,
    pub nodes: Vec<AdminNodeRow>,
    pub roles: Vec<(&'static str, &'static str)>,
    pub current_user_id: i64,
    pub list: bool,
    pub nodes_total: usize,
    pub silent: usize,
    pub open_alarms: i64,
    pub open_orders: i64,
    pub user_name: String,
    pub user_role: String,
    pub live: bool,
}

#[derive(Template)]
#[template(path = "drift.html")]
pub struct DriftTemplate {
    pub nodes_total: usize,
    pub silent: usize,
    pub open_alarms: i64,
    pub open_orders: i64,
    pub user_name: String,
    pub user_role: String,
    pub live: bool,
}

#[derive(Template)]
#[template(path = "fragments/ops_tiles.html")]
pub struct OpsTilesFragment {
    pub healthy: bool,
    pub problems: Vec<String>,
    pub window_seconds: i64,
    pub measurements: String,
    pub health: String,
    pub rejected: String,
    pub failed: String,
    pub http_requests: String,
    pub http_server_errors: String,
    pub http_mean_ms: String,
    pub http_slowest_ms: String,
    pub pool_size: usize,
    pub pool_available: usize,
    pub pool_waiting: usize,
    pub measurements_total: u64,
    pub registrations: u64,
    pub alarms_raised: u64,
    pub last_ingest: Stamp,
    pub last_detection: Stamp,
}

/// A move an operator or technician may make on an alarm.
#[derive(Debug, Clone, Copy)]
pub struct Transition {
    pub to: &'static str,
    pub label: &'static str,
}

/// One alarm as the feed draws it.
pub struct AlarmRow {
    pub id: i64,
    pub node_id: i64,
    pub node_name: String,
    pub metric: &'static str,
    pub state: &'static str,
    pub raised_at: Stamp,
    pub summary: String,
    pub actions: Vec<Transition>,
    pub may_dispatch: bool,
}

/// One alarm on its own, for a swap in place.
#[derive(Template)]
#[template(path = "fragments/alarm_entry.html")]
pub struct AlarmEntry {
    pub alarm: AlarmRow,
}

/// The backlog, drawn once when the dashboard loads.
#[derive(Template)]
#[template(path = "fragments/alarm_list.html")]
pub struct AlarmList {
    pub alarms: Vec<AlarmRow>,
}

#[derive(Template)]
#[template(path = "fragments/alarm_stub.html")]
pub struct AlarmStub {
    pub alarm_id: i64,
    pub replace: bool,
}

/// One reading from a node's own health sample, as a labelled bar.
pub struct Meter {
    pub label: &'static str,
    pub value: String,
    pub percent: f64,
    pub tone: &'static str,
}

/// The latest health sample a node sent.
pub struct HealthView {
    pub measured_at: Stamp,
    pub uptime: String,
    pub meters: Vec<Meter>,
}

/// One of the panel's span buttons, with its links already built.
pub struct SpanChoice {
    pub label: &'static str,
    pub url: String,
    pub fragment: String,
    pub current: bool,
}

/// One channel a node watches, and the metrics it gets a chart for.
pub struct ChannelView {
    pub id: i64,
    pub name: String,
    pub frequency: String,
    pub modulation: &'static str,
    pub metrics: Vec<&'static str>,
}

/// The slide-over that opens when a node is clicked.
#[derive(Template)]
#[template(path = "fragments/node_panel.html")]
pub struct NodePanel {
    pub id: i64,
    pub name: String,
    pub external_identity: String,
    pub rights: PanelRights,
    pub enrolled: bool,
    pub state: &'static str,
    pub at: Stamp,
    pub position: Option<String>,
    pub health: Option<HealthView>,
    pub channels: Vec<ChannelView>,
    pub hours: i64,
    pub spans: Vec<SpanChoice>,
    pub toggle: SpanChoice,
    pub show_all: bool,
}

/// What this viewer may do with the node in front of them.
pub struct PanelRights {
    pub edit: bool,
    pub inspect: bool,
}

/// One metric of one live dwell, already formatted.
pub struct LiveReading {
    pub name: &'static str,
    pub value: String,
}

/// One dwell, pushed out of band into an open inspect strip.
#[derive(Template)]
#[template(path = "fragments/live_readings.html")]
pub struct LiveReadings {
    pub node_id: i64,
    pub slug: String,
    pub label: String,
    pub at: Stamp,
    pub readings: Vec<LiveReading>,
}

/// One channel the inspect strip leaves a row for.
pub struct LiveChannel {
    pub name: String,
    pub slug: String,
}

/// Whether the node answered, on its own for a renewal.
#[derive(Template)]
#[template(path = "fragments/live_renewal.html")]
pub struct LiveRenewal {
    pub reachable: bool,
}

/// The inspect strip, opened from the panel's live toggle.
#[derive(Template)]
#[template(path = "fragments/live_panel.html")]
pub struct LivePanel {
    pub node_id: i64,
    pub channels: Vec<LiveChannel>,
    pub reachable: bool,
    pub renew_seconds: i64,
}

/// One work order as the list draws it.
pub struct WorkOrderRow {
    pub id: i64,
    pub alarm_id: i64,
    pub node_name: String,
    pub station_name: String,
    pub technician: String,
    pub status: &'static str,
    pub dispatched_at: Stamp,
    pub completed_at: Stamp,
    pub fault_present: Option<bool>,
    pub cause: String,
    pub action_taken: String,
    pub may_complete: bool,
}

/// Every work order this viewer may see.
#[derive(Template)]
#[template(path = "fragments/work_orders.html")]
pub struct WorkOrderList {
    pub orders: Vec<WorkOrderRow>,
}

/// One work order on its own, for a swap in place after a field report.
#[derive(Template)]
#[template(path = "fragments/work_order_entry.html")]
pub struct WorkOrderEntry {
    pub order: WorkOrderRow,
}

/// Somebody an alarm can be sent to.
pub struct Technician {
    pub id: i64,
    pub name: String,
}

/// The dispatch form, opened from an alarm entry.
#[derive(Template)]
#[template(path = "fragments/dispatch_form.html")]
pub struct DispatchForm {
    pub alarm_id: i64,
    pub technicians: Vec<Technician>,
    pub station_name: String,
}
