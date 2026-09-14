//! Async wrapper around the `cup` CLI. Every method shells out to `cup ... --json`
//! and parses the result into the types in `model`.

use std::process::Stdio;

use serde_json::Value;
use tokio::process::Command;

use crate::model::*;
use crate::text::strip_ansi;

#[derive(Clone, Debug)]
pub struct CupClient {
    pub bin: String,
    pub profile: Option<String>,
}

pub type CupResult<T> = Result<T, String>;

impl CupClient {
    pub fn new(bin: String, profile: Option<String>) -> Self {
        Self { bin, profile }
    }

    fn command(&self, args: &[String]) -> Command {
        let mut cmd = Command::new(&self.bin);
        if let Some(p) = &self.profile {
            cmd.arg("-p").arg(p);
        }
        cmd.args(args);
        cmd.env("NO_COLOR", "1");
        cmd.env("FORCE_COLOR", "0");
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.kill_on_drop(true);
        cmd
    }

    /// Run a command and return trimmed stdout. Non-zero exit becomes `Err`.
    pub async fn run_raw(&self, args: &[String]) -> CupResult<String> {
        let out = self
            .command(args)
            .output()
            .await
            .map_err(|e| format!("failed to run `{}`: {e}", self.bin))?;
        let stdout = strip_ansi(&String::from_utf8_lossy(&out.stdout));
        let stderr = strip_ansi(&String::from_utf8_lossy(&out.stderr));
        if out.status.success() {
            Ok(stdout.trim().to_string())
        } else {
            let msg = if !stderr.trim().is_empty() { stderr } else { stdout };
            let msg = msg.trim();
            let first = msg.lines().find(|l| !l.trim().is_empty()).unwrap_or("command failed");
            Err(first.trim().to_string())
        }
    }

    /// Run a read command with `--json` appended and parse the output.
    pub async fn run_json(&self, args: &[&str]) -> CupResult<Value> {
        let mut a: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        a.push("--json".into());
        let text = self.run_raw(&a).await?;
        if text.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).map_err(|e| {
            let head: String = text.lines().next().unwrap_or("").chars().take(120).collect();
            format!("bad JSON from cup ({e}): {head}")
        })
    }

    /// Run a write command (already fully specified). Returns stdout.
    pub async fn run_write(&self, args: &[String]) -> CupResult<String> {
        self.run_raw(args).await
    }

    pub async fn auth(&self) -> CupResult<User> {
        let v = self.run_json(&["auth"]).await?;
        let user = v.get("user").cloned().unwrap_or(v);
        serde_json::from_value(user).map_err(|e| format!("auth parse: {e}"))
    }

    pub async fn spaces(&self) -> CupResult<Vec<Space>> {
        let v = self.run_json(&["spaces"]).await?;
        serde_json::from_value(v).map_err(|e| format!("spaces parse: {e}"))
    }

    pub async fn space_children(&self, space_id: &str) -> CupResult<SpaceChildren> {
        let folder_args = ["folders", space_id];
        let list_args = ["lists", space_id];
        let (folders, lists) = tokio::join!(self.run_json(&folder_args), self.run_json(&list_args));
        let folders: Vec<Folder> =
            serde_json::from_value(folders?).map_err(|e| format!("folders parse: {e}"))?;
        let flat: Vec<FlatList> =
            serde_json::from_value(lists?).map_err(|e| format!("lists parse: {e}"))?;
        let folderless = flat
            .into_iter()
            .filter(|l| matches!(l.folder.as_deref(), None | Some("(none)") | Some("")))
            .map(|l| ListInfo { id: l.id, name: l.name, task_count: None })
            .collect();
        Ok(SpaceChildren { folders, lists: folderless })
    }

    pub async fn tasks(&self, source: &TaskSource, include_closed: bool) -> CupResult<Vec<TaskSummary>> {
        let mut args: Vec<&str> = Vec::new();
        let query;
        match source {
            TaskSource::List { id, .. } => {
                args.extend(["tasks", "--list", id.as_str(), "--all"]);
                if include_closed {
                    args.push("--include-closed");
                }
            }
            TaskSource::Assigned => args.push("assigned"),
            TaskSource::Inbox => args.push("inbox"),
            TaskSource::Overdue => args.push("overdue"),
            TaskSource::Search { query: q, scope } => {
                query = q.clone();
                args.extend(["search", query.as_str()]);
                match scope {
                    SearchScope::Space { id, .. } => args.extend(["--all", "--space", id.as_str()]),
                    SearchScope::Mine => {}
                    SearchScope::Workspace => args.push("--all"),
                }
                if include_closed {
                    args.push("--include-closed");
                }
            }
        }
        let v = self.run_json(&args).await?;
        let items = extract_task_array(v);
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            if let Some(t) = TaskSummary::from_cup(&item) {
                if seen.insert(t.id.clone()) {
                    out.push(t);
                }
            }
        }
        Ok(out)
    }

    /// Task detail plus comments in a single round trip.
    pub async fn activity(&self, task_id: &str) -> CupResult<(TaskDetail, Vec<Comment>)> {
        let v = self.run_json(&["activity", task_id]).await?;
        let task = v.get("task").cloned().unwrap_or(Value::Null);
        let comments = v.get("comments").cloned().unwrap_or(Value::Array(vec![]));
        let task: TaskDetail =
            serde_json::from_value(task).map_err(|e| format!("task parse: {e}"))?;
        let comments: Vec<Comment> = serde_json::from_value(comments).unwrap_or_default();
        Ok((task, comments))
    }

    pub async fn subtasks(&self, task_id: &str) -> CupResult<Vec<TaskSummary>> {
        let v = self.run_json(&["subtasks", task_id, "--include-closed"]).await?;
        let items = extract_task_array(v);
        Ok(items
            .iter()
            .filter_map(TaskSummary::from_cup)
            // cup returns siblings when asked about a subtask; keep real children only
            .filter(|t| t.parent.as_deref() == Some(task_id))
            .collect())
    }

    pub async fn members(&self) -> CupResult<Vec<User>> {
        let v = self.run_json(&["members"]).await?;
        let items = match v {
            Value::Array(a) => a,
            Value::Object(mut o) => o.remove("members").and_then(|m| m.as_array().cloned()).unwrap_or_default(),
            _ => vec![],
        };
        Ok(items
            .into_iter()
            .filter_map(|i| serde_json::from_value::<User>(i).ok())
            .filter(|u| !u.display().is_empty())
            .collect())
    }
}
