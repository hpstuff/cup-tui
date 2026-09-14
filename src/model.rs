//! Data types mirroring the JSON emitted by the `cup` CLI.
//!
//! Everything is lenient (`#[serde(default)]`, `Option`, `Value`) so a field
//! that changes shape in a future cup release degrades to "unknown" instead of
//! failing the whole parse.

use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize, Clone, Debug, Default)]
pub struct User {
    #[serde(default)]
    pub id: Value,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
}

impl User {
    pub fn id_string(&self) -> String {
        match &self.id {
            Value::Number(n) => n.to_string(),
            Value::String(s) => s.clone(),
            _ => String::new(),
        }
    }

    pub fn display(&self) -> String {
        self.username
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| self.email.clone())
            .unwrap_or_else(|| self.id_string())
    }
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct StatusDef {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub orderindex: Value,
    #[serde(default)]
    pub color: Option<String>,
}

impl StatusDef {
    pub fn order(&self) -> f64 {
        value_f64(&self.orderindex).unwrap_or(0.0)
    }
}

#[derive(Deserialize, Clone, Debug)]
pub struct Space {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub statuses: Vec<StatusDef>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct ListInfo {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub task_count: Option<i64>,
}

/// Shape returned by `cup lists <space>` (flat, folder given as a name).
#[derive(Deserialize, Clone, Debug)]
pub struct FlatList {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub folder: Option<String>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Folder {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub lists: Vec<ListInfo>,
}

#[derive(Clone, Debug, Default)]
pub struct SpaceChildren {
    pub folders: Vec<Folder>,
    /// Lists that live directly in the space (no folder).
    pub lists: Vec<ListInfo>,
}

/// One row of a task list. Built either from cup's compact summary
/// (`from_cup`, no assignees) or from a raw ClickUp API task (`from_api`).
#[derive(Clone, Debug, Default)]
pub struct TaskSummary {
    pub id: String,
    pub name: String,
    pub status: String,
    pub status_color: Option<String>,
    pub status_order: Option<f64>,
    /// ClickUp status type: open / custom / done / closed (API only).
    pub status_type: Option<String>,
    pub task_type: String,
    pub priority: String,
    pub due_raw: Option<String>,
    pub list: String,
    pub url: String,
    pub parent: Option<String>,
    pub assignees: Vec<User>,
    /// True when the source knows about assignees (API); cup summaries don't.
    pub has_assignee_data: bool,
}

impl TaskSummary {
    pub fn due_ms(&self) -> Option<i64> {
        self.due_raw.as_deref().and_then(|s| s.parse().ok())
    }

    /// Coarse ordering of status kinds: open, custom, done, closed.
    pub fn type_rank(&self) -> f64 {
        match self.status_type.as_deref() {
            Some("open") => 0.0,
            Some("unstarted") => 0.5,
            Some("custom") => 1.0,
            Some("done") => 2.0,
            Some("closed") => 3.0,
            _ => {
                if crate::text::is_done_status(&self.status) {
                    2.0
                } else {
                    1.0
                }
            }
        }
    }

    /// Done or closed according to the status type, falling back to the name.
    pub fn is_done(&self) -> bool {
        match self.status_type.as_deref() {
            Some("done") | Some("closed") => true,
            Some(_) => false,
            None => crate::text::is_done_status(&self.status),
        }
    }

    pub fn assignee_ids(&self) -> Vec<String> {
        self.assignees.iter().map(|u| u.id_string()).collect()
    }

    fn str_field(v: &Value, key: &str) -> String {
        v.get(key).and_then(|x| x.as_str()).unwrap_or("").to_string()
    }

    /// From `cup tasks/assigned/inbox/search --json` items.
    pub fn from_cup(v: &Value) -> Option<Self> {
        let id = v.get("id")?.as_str()?.to_string();
        Some(Self {
            id,
            name: Self::str_field(v, "name"),
            status: Self::str_field(v, "status"),
            status_color: None,
            status_order: None,
            status_type: None,
            task_type: Self::str_field(v, "task_type"),
            priority: Self::str_field(v, "priority"),
            due_raw: v.get("dueRaw").and_then(|x| x.as_str()).map(str::to_string),
            list: Self::str_field(v, "list"),
            url: Self::str_field(v, "url"),
            parent: v.get("parent").and_then(|x| x.as_str()).map(str::to_string),
            assignees: Vec::new(),
            has_assignee_data: false,
        })
    }

    /// From a raw ClickUp API task object.
    pub fn from_api(v: &Value, types: &std::collections::HashMap<i64, String>) -> Option<Self> {
        let id = v.get("id")?.as_str()?.to_string();
        let status = v.get("status").cloned().unwrap_or(Value::Null);
        let type_id = v.get("custom_item_id").and_then(value_i64).unwrap_or(0);
        let task_type = if type_id == 0 {
            "task".to_string()
        } else {
            types.get(&type_id).cloned().unwrap_or_else(|| format!("type_{type_id}"))
        };
        let assignees: Vec<User> = v
            .get("assignees")
            .and_then(|a| a.as_array())
            .map(|a| a.iter().filter_map(|u| serde_json::from_value(u.clone()).ok()).collect())
            .unwrap_or_default();
        Some(Self {
            id,
            name: Self::str_field(v, "name"),
            status: Self::str_field(&status, "status"),
            status_color: status.get("color").and_then(|x| x.as_str()).map(str::to_string),
            status_order: status.get("orderindex").and_then(value_f64),
            status_type: status.get("type").and_then(|x| x.as_str()).map(str::to_string),
            task_type,
            priority: v
                .get("priority")
                .and_then(|p| p.get("priority"))
                .and_then(|x| x.as_str())
                .unwrap_or("none")
                .to_string(),
            due_raw: v.get("due_date").and_then(|x| x.as_str()).map(str::to_string),
            list: v.get("list").map(|l| Self::str_field(l, "name")).unwrap_or_default(),
            url: Self::str_field(v, "url"),
            parent: v.get("parent").and_then(|x| x.as_str()).map(str::to_string),
            assignees,
            has_assignee_data: true,
        })
    }
}

/// Structured filters on the task table. Statuses and assignees are also sent
/// to the API when the source supports it; priorities are client-side only.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct TaskFilters {
    pub statuses: Vec<String>,
    pub priorities: Vec<String>,
    /// User ids, or "none" for unassigned.
    pub assignees: Vec<String>,
}

impl TaskFilters {
    pub fn is_empty(&self) -> bool {
        self.statuses.is_empty() && self.priorities.is_empty() && self.assignees.is_empty()
    }

    /// The part of the filter the server applies; changing it needs a refetch.
    pub fn server_key(&self) -> String {
        let mut st = self.statuses.clone();
        st.sort();
        let mut asg: Vec<&String> = self.assignees.iter().filter(|a| a.as_str() != "none").collect();
        asg.sort();
        format!("st={}|as={}", st.join(","), asg.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(","))
    }

    pub fn server_assignees(&self) -> Vec<String> {
        self.assignees.iter().filter(|a| a.as_str() != "none").cloned().collect()
    }

    /// Client-side check. `assignee_data` says whether rows carry assignees;
    /// without it the assignee filter is skipped rather than hiding everything.
    pub fn matches(&self, t: &TaskSummary, assignee_data: bool) -> bool {
        if !self.statuses.is_empty() && !self.statuses.iter().any(|s| s.trim().eq_ignore_ascii_case(t.status.trim())) {
            return false;
        }
        if !self.priorities.is_empty() {
            let p = if t.priority.is_empty() { "none" } else { t.priority.as_str() };
            if !self.priorities.iter().any(|x| x.eq_ignore_ascii_case(p)) {
                return false;
            }
        }
        if !self.assignees.is_empty() && assignee_data {
            let ids = t.assignee_ids();
            let want_none = self.assignees.iter().any(|a| a == "none");
            let hit = (want_none && ids.is_empty()) || self.assignees.iter().any(|a| ids.contains(a));
            if !hit {
                return false;
            }
        }
        true
    }

    pub fn summary(&self, me: Option<&str>) -> String {
        let mut parts = Vec::new();
        if !self.statuses.is_empty() {
            parts.push(if self.statuses.len() == 1 { self.statuses[0].clone() } else { format!("{} statuses", self.statuses.len()) });
        }
        if !self.priorities.is_empty() {
            parts.push(format!("pri:{}", self.priorities.join("/")));
        }
        if !self.assignees.is_empty() {
            let mut a = Vec::new();
            for id in &self.assignees {
                if Some(id.as_str()) == me {
                    a.push("me".to_string());
                } else if id == "none" {
                    a.push("unassigned".to_string());
                } else {
                    a.push(format!("#{id}"));
                }
            }
            parts.push(format!("@{}", a.join(",")));
        }
        parts.join(" · ")
    }
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct StatusObj {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct PriorityObj {
    #[serde(default)]
    pub priority: String,
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct Tag {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub tag_bg: Option<String>,
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct Ref {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct TaskDetail {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub custom_id: Option<String>,
    #[serde(default)]
    pub status: Option<StatusObj>,
    #[serde(default)]
    pub priority: Option<PriorityObj>,
    #[serde(default)]
    pub assignees: Vec<User>,
    #[serde(default)]
    pub creator: Option<User>,
    #[serde(default)]
    pub tags: Vec<Tag>,
    #[serde(default)]
    pub due_date: Option<String>,
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub date_created: Option<String>,
    #[serde(default)]
    pub date_updated: Option<String>,
    #[serde(default)]
    pub list: Option<Ref>,
    #[serde(default)]
    pub folder: Option<Ref>,
    #[serde(default)]
    pub space: Option<Ref>,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub markdown_description: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub text_content: Option<String>,
    #[serde(default)]
    pub time_estimate: Value,
    #[serde(default)]
    pub time_spent: Value,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub custom_fields: Vec<Value>,
    #[serde(default)]
    pub checklists: Vec<Value>,
    #[serde(default)]
    pub attachments: Vec<Value>,
    #[serde(default)]
    pub archived: bool,
}

impl TaskDetail {
    pub fn status_name(&self) -> String {
        self.status.as_ref().map(|s| s.status.clone()).unwrap_or_default()
    }

    pub fn priority_name(&self) -> Option<String> {
        self.priority
            .as_ref()
            .map(|p| p.priority.clone())
            .filter(|p| !p.is_empty() && p != "none")
    }

    /// Best available description body, preferring markdown.
    pub fn body(&self) -> String {
        for candidate in [&self.markdown_description, &self.description, &self.text_content] {
            if let Some(s) = candidate {
                if !s.trim().is_empty() {
                    return s.clone();
                }
            }
        }
        String::new()
    }

    pub fn has_assignee(&self, user_id: &str) -> bool {
        self.assignees.iter().any(|u| u.id_string() == user_id)
    }
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct Comment {
    #[serde(default)]
    pub user: Value,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub text: String,
}

impl Comment {
    pub fn author(&self) -> String {
        match &self.user {
            Value::String(s) => s.clone(),
            Value::Object(o) => o
                .get("username")
                .and_then(|v| v.as_str())
                .or_else(|| o.get("email").and_then(|v| v.as_str()))
                .unwrap_or("?")
                .to_string(),
            _ => "?".to_string(),
        }
    }
}

/// What the task table is currently showing.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TaskSource {
    List {
        id: String,
        name: String,
        space_id: Option<String>,
        folder_name: Option<String>,
        space_name: Option<String>,
    },
    Assigned,
    Inbox,
    Overdue,
    Search { query: String, scope: SearchScope },
}

/// Where a search looks. Whole-workspace searches walk every list through the
/// cup CLI and can take over a minute, so they are opt-in.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SearchScope {
    Space { id: String, name: String },
    Mine,
    Workspace,
}

impl SearchScope {
    pub fn label(&self) -> String {
        match self {
            SearchScope::Space { name, .. } => name.clone(),
            SearchScope::Mine => "my tasks".into(),
            SearchScope::Workspace => "whole workspace".into(),
        }
    }
}

impl TaskSource {
    /// Cache key. Includes the closed-tasks toggle where the CLI supports it.
    pub fn key(&self, include_closed: bool) -> String {
        match self {
            TaskSource::List { id, .. } => format!("list:{id}:{}", include_closed as u8),
            TaskSource::Assigned => "assigned".into(),
            TaskSource::Inbox => "inbox".into(),
            TaskSource::Overdue => "overdue".into(),
            TaskSource::Search { query, scope } => {
                let sc = match scope {
                    SearchScope::Space { id, .. } => format!("space={id}"),
                    SearchScope::Mine => "mine".into(),
                    SearchScope::Workspace => "all".into(),
                };
                format!("search:{query}:{sc}:{}", include_closed as u8)
            }
        }
    }

    pub fn title(&self) -> String {
        match self {
            TaskSource::List { name, .. } => name.clone(),
            TaskSource::Assigned => "My tasks".into(),
            TaskSource::Inbox => "Inbox".into(),
            TaskSource::Overdue => "Overdue".into(),
            TaskSource::Search { query, .. } => format!("Search: {query}"),
        }
    }

    pub fn breadcrumb(&self) -> Vec<String> {
        match self {
            TaskSource::List {
                name,
                folder_name,
                space_name,
                ..
            } => {
                let mut v = Vec::new();
                if let Some(s) = space_name {
                    v.push(s.clone());
                }
                if let Some(f) = folder_name {
                    v.push(f.clone());
                }
                v.push(name.clone());
                v
            }
            TaskSource::Search { query, scope } => vec![scope.label(), format!("search \"{query}\"")],
            other => vec![other.title()],
        }
    }

    pub fn supports_closed_toggle(&self) -> bool {
        matches!(self, TaskSource::List { .. } | TaskSource::Search { .. } | TaskSource::Assigned)
    }

    pub fn list_id(&self) -> Option<&str> {
        match self {
            TaskSource::List { id, .. } => Some(id),
            _ => None,
        }
    }

    pub fn space_id(&self) -> Option<&str> {
        match self {
            TaskSource::List { space_id, .. } => space_id.as_deref(),
            TaskSource::Search { scope: SearchScope::Space { id, .. }, .. } => Some(id),
            _ => None,
        }
    }
}

pub fn value_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

pub fn value_i64(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// Pull a task array out of whatever container cup used: a bare array, an
/// object with a `tasks` key, or an object whose values are arrays (grouped
/// output such as `cup assigned` / `cup inbox`).
pub fn extract_task_array(v: Value) -> Vec<Value> {
    match v {
        Value::Array(a) => a,
        Value::Object(mut o) => {
            if let Some(Value::Array(a)) = o.remove("tasks") {
                return a;
            }
            let mut out = Vec::new();
            for (_, val) in o {
                if let Value::Array(a) = val {
                    out.extend(a);
                }
            }
            out
        }
        _ => Vec::new(),
    }
}

/// Human-readable rendering of a ClickUp custom field, best effort.
pub fn custom_field_display(f: &Value) -> Option<(String, String)> {
    let name = f.get("name")?.as_str()?.to_string();
    let value = f.get("value")?;
    let kind = f.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let text = match (kind, value) {
        (_, Value::Null) => return None,
        ("drop_down", Value::Number(n)) => {
            let idx = n.as_u64()? as usize;
            f.get("type_config")?
                .get("options")?
                .as_array()?
                .iter()
                .find(|o| o.get("orderindex").and_then(|x| x.as_u64()) == Some(idx as u64))
                .or_else(|| f["type_config"]["options"].as_array()?.get(idx))
                .and_then(|o| o.get("name").and_then(|x| x.as_str()))
                .unwrap_or("?")
                .to_string()
        }
        ("labels", Value::Array(ids)) => {
            let opts = f["type_config"]["options"].as_array().cloned().unwrap_or_default();
            ids.iter()
                .map(|id| {
                    opts.iter()
                        .find(|o| o.get("id") == Some(id))
                        .and_then(|o| o.get("label").and_then(|x| x.as_str()))
                        .unwrap_or("?")
                        .to_string()
                })
                .collect::<Vec<_>>()
                .join(", ")
        }
        ("users", Value::Array(us)) => us
            .iter()
            .map(|u| u.get("username").and_then(|x| x.as_str()).unwrap_or("?").to_string())
            .collect::<Vec<_>>()
            .join(", "),
        ("tasks", Value::Array(ts)) => ts
            .iter()
            .map(|t| {
                t.get("name")
                    .and_then(|x| x.as_str())
                    .or_else(|| t.get("id").and_then(|x| x.as_str()))
                    .unwrap_or("?")
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join(", "),
        ("date", v) => value_i64(v)
            .map(crate::text::fmt_ms_date)
            .unwrap_or_else(|| v.to_string()),
        ("checkbox", v) => {
            if v.as_bool() == Some(true) || v.as_str() == Some("true") {
                "☑".into()
            } else {
                "☐".into()
            }
        }
        (_, Value::String(s)) => s.clone(),
        (_, Value::Number(n)) => n.to_string(),
        (_, Value::Bool(b)) => b.to_string(),
        (_, Value::Array(a)) => a
            .iter()
            .map(|x| match x {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .collect::<Vec<_>>()
            .join(", "),
        (_, other) => other.to_string(),
    };
    if text.trim().is_empty() {
        return None;
    }
    Some((name, text))
}
