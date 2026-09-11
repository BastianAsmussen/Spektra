use std::collections::HashSet;

use diesel::prelude::*;

use super::errors::ApiError;
use crate::db::schema::{alarms, nodes, roles, users, work_orders};
use crate::state::AppState;

/// Role name that only sees the nodes it has been dispatched to.
pub const TECHNICIAN: &str = "technician";

/// Role name that may change alarm state and dispatch work orders.
pub const OPERATOR: &str = "operator";

/// Role name with full access.
pub const ADMINISTRATOR: &str = "administrator";

/// Role name with read access and nothing else.
pub const READER: &str = "reader";

/// The nodes one user may see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Visibility {
    /// Administrators, operators and readers watch the whole fleet.
    Fleet,
    /// A technician watches the nodes behind their own work orders.
    Nodes(HashSet<i64>),
}

impl Visibility {
    /// Whether this user may see anything about `node_id`.
    #[must_use]
    pub fn allows(&self, node_id: i64) -> bool {
        match *self {
            Self::Fleet => true,
            Self::Nodes(ref allowed) => allowed.contains(&node_id),
        }
    }

    /// The node ids to filter a query by, or `None` for the whole fleet.
    #[must_use]
    pub fn node_filter(&self) -> Option<Vec<i64>> {
        match *self {
            Self::Fleet => None,
            Self::Nodes(ref allowed) => Some(allowed.iter().copied().collect()),
        }
    }
}

/// A user's role name and what they may see.
#[derive(Debug, Clone)]
pub struct Access {
    pub user_id: i64,
    pub role: String,
    pub visibility: Visibility,
}

impl Access {
    /// Whether this user may move an alarm along its lifecycle or dispatch a work order.
    #[must_use]
    pub fn may_act(&self) -> bool {
        self.role != READER
    }

    /// Whether this user may dispatch work orders.
    #[must_use]
    pub fn may_dispatch(&self) -> bool {
        self.role == OPERATOR || self.role == ADMINISTRATOR
    }

    /// Whether this user may file the field report on one work order.
    #[must_use]
    pub fn may_complete(&self, assignee: i64) -> bool {
        self.may_act() && (assignee == self.user_id || self.is_admin())
    }

    /// Whether a work order listing should be narrowed to this user's own.
    #[must_use]
    pub fn sees_only_own_orders(&self) -> bool {
        self.role == TECHNICIAN
    }

    /// Whether this user administers users and nodes.
    #[must_use]
    pub fn is_admin(&self) -> bool {
        self.role == ADMINISTRATOR
    }
}

/// Whether this user may change one node's record: an administrator, or its owner.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] when there is no such node.
pub async fn may_edit_node(
    state: &AppState,
    access: &Access,
    node_id: i64,
) -> Result<bool, ApiError> {
    if access.is_admin() {
        return Ok(true);
    }

    let user_id = access.user_id;
    let conn = state.pool.get().await?;
    let owner: Option<i64> = conn
        .interact(move |conn| {
            nodes::table
                .filter(nodes::id.eq(node_id))
                .select(nodes::owner_id)
                .first(conn)
        })
        .await??;

    Ok(owner == Some(user_id))
}

/// Resolve one user's role and node set, in one round trip.
///
/// # Errors
///
/// Returns [`ApiError::Unauthorized`] when the session points at a missing user.
pub async fn resolve(state: &AppState, user_id: i64) -> Result<Access, ApiError> {
    let conn = state.pool.get().await?;
    conn.interact(move |conn| {
        let role: String = users::table
            .inner_join(roles::table)
            .filter(users::id.eq(user_id))
            .select(roles::name)
            .first(conn)?;

        if role != TECHNICIAN {
            return Ok(Access {
                user_id,
                role,
                visibility: Visibility::Fleet,
            });
        }

        let dispatched: Vec<i64> = work_orders::table
            .inner_join(alarms::table)
            .filter(work_orders::technician_user_id.eq(user_id))
            .select(alarms::node_id)
            .distinct()
            .load(conn)?;

        Ok(Access {
            user_id,
            role,
            visibility: Visibility::Nodes(dispatched.into_iter().collect()),
        })
    })
    .await?
    .map_err(|err: diesel::result::Error| match err {
        diesel::result::Error::NotFound => {
            ApiError::Unauthorized("The session points at a user that no longer exists.".into())
        }
        other => ApiError::internal(other),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fleet_view_allows_every_node() {
        let visibility = Visibility::Fleet;

        assert!(visibility.allows(1));
        assert!(visibility.allows(9_999));
        assert_eq!(visibility.node_filter(), None);
    }

    #[test]
    fn a_technician_only_sees_their_own_nodes() {
        let visibility = Visibility::Nodes([1_i64, 2].into_iter().collect());

        assert!(visibility.allows(1));
        assert!(visibility.allows(2));
        assert!(!visibility.allows(3));

        let mut filter = visibility.node_filter().expect("a filtered view");
        filter.sort_unstable();
        assert_eq!(filter, vec![1, 2]);
    }

    #[test]
    fn a_technician_with_no_dispatches_sees_nothing() {
        let visibility = Visibility::Nodes(HashSet::new());

        assert!(!visibility.allows(1));
        assert_eq!(visibility.node_filter(), Some(Vec::new()));
    }

    #[test]
    fn a_reader_may_not_act() {
        let access = |role: &str| Access {
            user_id: 1,
            role: role.to_owned(),
            visibility: Visibility::Fleet,
        };

        assert!(!access(READER).may_act());
        assert!(access(OPERATOR).may_act());
        assert!(access(TECHNICIAN).may_act());
        assert!(access(ADMINISTRATOR).may_act());
    }

    #[test]
    fn only_operators_and_administrators_dispatch() {
        let access = |role: &str| Access {
            user_id: 1,
            role: role.to_owned(),
            visibility: Visibility::Fleet,
        };

        assert!(access(OPERATOR).may_dispatch());
        assert!(access(ADMINISTRATOR).may_dispatch());
        assert!(!access(TECHNICIAN).may_dispatch());
        assert!(!access(READER).may_dispatch());
    }

    #[test]
    fn only_the_assignee_or_an_administrator_may_complete() {
        let access = |role: &str| Access {
            user_id: 1,
            role: role.to_owned(),
            visibility: Visibility::Fleet,
        };

        assert!(access(TECHNICIAN).may_complete(1));
        assert!(!access(TECHNICIAN).may_complete(2));
        assert!(access(ADMINISTRATOR).may_complete(2));
        assert!(!access(OPERATOR).may_complete(2));
        assert!(!access(READER).may_complete(1));
    }

    #[test]
    fn only_an_administrator_administers() {
        let access = |role: &str| Access {
            user_id: 1,
            role: role.to_owned(),
            visibility: Visibility::Fleet,
        };

        assert!(access(ADMINISTRATOR).is_admin());
        assert!(!access(OPERATOR).is_admin());
        assert!(!access(TECHNICIAN).is_admin());
        assert!(!access(READER).is_admin());
    }
}
