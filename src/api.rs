//! Minimal direct ClickUp API client, used only where the cup CLI cannot
//! help: paginated task lists with assignees and server-side filters.
//! Authentication reuses cup's own config (`~/.config/cup/config.json`,
//! profiles, `CU_API_TOKEN` / `CU_TEAM_ID`), so there is nothing extra to set up.

use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::Value;
use tokio::sync::OnceCell;

use crate::model::{value_i64, TaskFilters, TaskSummary};

const BASE: &str = "https://api.clickup.com/api/v2";

pub struct ApiClient {
    token: String,
    pub team_id: String,
    http: reqwest::Client,
    types: OnceCell<HashMap<i64, String>>,
}

pub struct Page {
    pub tasks: Vec<TaskSummary>,
    pub last_page: bool,
}

fn config_path() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.trim().is_empty() {
            return PathBuf::from(xdg).join("cup").join("config.json");
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config").join("cup").join("config.json")
}

impl ApiClient {
    /// Resolve token + team the same way cup does.
    pub fn from_cup_config(profile: Option<&str>) -> Result<Self, String> {
        let env_token = std::env::var("CU_API_TOKEN").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let env_team = std::env::var("CU_TEAM_ID").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let (token, team_id) = match (env_token, env_team) {
            (Some(t), Some(team)) => (t, team),
            _ => {
                let path = config_path();
                let raw = std::fs::read_to_string(&path)
                    .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
                let cfg: Value = serde_json::from_str(&raw).map_err(|e| format!("bad cup config: {e}"))?;
                let prof = if let Some(profiles) = cfg.get("profiles").and_then(|p| p.as_object()) {
                    let name = profile
                        .map(str::to_string)
                        .or_else(|| std::env::var("CU_PROFILE").ok().filter(|s| !s.trim().is_empty()))
                        .or_else(|| cfg.get("defaultProfile").and_then(|d| d.as_str()).map(str::to_string))
                        .ok_or("cup config has no default profile")?;
                    profiles.get(&name).cloned().ok_or(format!("cup profile `{name}` not found"))?
                } else {
                    cfg.clone()
                };
                let t = prof.get("apiToken").and_then(|x| x.as_str()).ok_or("cup config has no apiToken")?.to_string();
                let team = prof.get("teamId").and_then(|x| x.as_str()).ok_or("cup config has no teamId")?.to_string();
                (t, team)
            }
        };
        if !token.starts_with("pk_") {
            return Err("cup apiToken does not look like a personal token (pk_…)".into());
        }
        let http = reqwest::Client::builder()
            .user_agent(concat!("cup-tui/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { token, team_id, http, types: OnceCell::new() })
    }

    async fn get(&self, path: &str, query: &[(String, String)]) -> Result<Value, String> {
        let resp = self
            .http
            .get(format!("{BASE}{path}"))
            .header("Authorization", &self.token)
            .query(query)
            .send()
            .await
            .map_err(|e| format!("ClickUp API: {e}"))?;
        let status = resp.status();
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            let err = body
                .get("err")
                .and_then(|e| e.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| body.to_string());
            return Err(format!("ClickUp API {}: {}", status.as_u16(), crate::text::truncate(&err, 120)));
        }
        Ok(body)
    }

    /// Custom task type names (Bug, Initiative, …), fetched once.
    pub async fn types(&self) -> &HashMap<i64, String> {
        self.types
            .get_or_init(|| async {
                let mut map = HashMap::new();
                if let Ok(v) = self.get(&format!("/team/{}/custom_item", self.team_id), &[]).await {
                    if let Some(items) = v.get("custom_items").and_then(|c| c.as_array()) {
                        for it in items {
                            if let (Some(id), Some(name)) = (it.get("id").and_then(value_i64), it.get("name").and_then(|n| n.as_str())) {
                                map.insert(id, name.to_string());
                            }
                        }
                    }
                }
                map
            })
            .await
    }

    fn filter_query(filters: &TaskFilters, q: &mut Vec<(String, String)>) {
        for s in &filters.statuses {
            q.push(("statuses[]".into(), s.clone()));
        }
        for a in filters.server_assignees() {
            q.push(("assignees[]".into(), a));
        }
    }

    fn parse_page(&self, v: Value, types: &HashMap<i64, String>) -> Page {
        let tasks = v
            .get("tasks")
            .and_then(|t| t.as_array())
            .map(|a| a.iter().filter_map(|t| TaskSummary::from_api(t, types)).collect())
            .unwrap_or_default();
        let last_page = v.get("last_page").and_then(|l| l.as_bool()).unwrap_or(true);
        Page { tasks, last_page }
    }

    /// One page (100 tasks) of a list.
    pub async fn list_tasks(&self, list_id: &str, page: u32, include_closed: bool, filters: &TaskFilters) -> Result<Page, String> {
        let mut q = vec![("page".to_string(), page.to_string()), ("subtasks".to_string(), "true".to_string())];
        if include_closed {
            q.push(("include_closed".into(), "true".into()));
        }
        Self::filter_query(filters, &mut q);
        let types = self.types().await.clone();
        let v = self.get(&format!("/list/{list_id}/task"), &q).await?;
        Ok(self.parse_page(v, &types))
    }

    /// One page of tasks across the workspace, for the given assignees.
    pub async fn team_tasks(&self, page: u32, assignees: &[String], include_closed: bool, filters: &TaskFilters) -> Result<Page, String> {
        let mut q = vec![
            ("page".to_string(), page.to_string()),
            ("subtasks".to_string(), "true".to_string()),
            ("order_by".to_string(), "updated".to_string()),
            ("reverse".to_string(), "true".to_string()),
        ];
        for a in assignees {
            q.push(("assignees[]".into(), a.clone()));
        }
        if include_closed {
            q.push(("include_closed".into(), "true".into()));
        }
        let mut f = filters.clone();
        f.assignees.clear(); // the view already fixes the assignee
        Self::filter_query(&f, &mut q);
        let types = self.types().await.clone();
        let v = self.get(&format!("/team/{}/task", self.team_id), &q).await?;
        Ok(self.parse_page(v, &types))
    }
}
