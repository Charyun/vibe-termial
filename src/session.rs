//! Workspace and terminal lifecycle, independent of any GUI toolkit.
use crate::{
    model::{AppData, CliKind, CliProfile, Tab},
    store::Store,
    terminal::Terminal,
};
use anyhow::Result;
use std::{
    collections::HashMap,
    fs::File,
    path::Path,
    sync::{Arc, atomic::Ordering},
};
use uuid::Uuid;

pub struct Session {
    pub data: AppData,
    pub store: Store,
    pub terminals: HashMap<Uuid, Terminal>,
    pub error: Option<String>,
    pub grid: (u16, u16),
    pub wake: Arc<dyn Fn() + Send + Sync>,
    pub read_only: bool,
    _lock: Option<File>,
    stopped: bool,
}
impl Session {
    pub fn new(store: Store, data: AppData, lock: Option<File>) -> Self {
        Self {
            data,
            store,
            terminals: HashMap::new(),
            error: None,
            grid: (30, 100),
            wake: Arc::new(|| {}),
            read_only: false,
            _lock: lock,
            stopped: false,
        }
    }
    pub fn selected(&self) -> Option<Uuid> {
        self.data.workspace().and_then(|w| w.selected_tab)
    }
    pub fn result(&mut self, result: Result<()>) {
        if let Err(e) = result {
            self.error = Some(format!("{e:#}"));
        }
    }
    pub fn persist(&mut self) {
        if !self.read_only {
            let r = self.store.save(&self.data);
            self.result(r);
        }
    }
    pub fn add_workspace(&mut self, path: &Path) {
        match self.data.add_workspace(path) {
            Ok(id) => self.select_workspace(id),
            Err(e) => self.error = Some(format!("{e:#}")),
        }
    }
    pub fn select_workspace(&mut self, id: Uuid) {
        self.data.selected_workspace = Some(id);
        self.terminals.retain(|_, t| t.loaded());
        self.ensure_history();
        self.persist();
    }
    pub fn select_tab(&mut self, id: Uuid) {
        if let Some(terminal) = self.terminals.get(&id)
            && let Ok(mut buffer) = terminal.buffer.lock()
        {
            buffer.parser.callbacks_mut().activity.acknowledge();
        }
        if let Some(w) = self.data.workspace_mut() {
            w.selected_tab = Some(id);
        }
        self.terminals.retain(|key, t| *key == id || t.loaded());
        self.ensure_history();
        self.persist();
    }
    pub fn ensure_history(&mut self) {
        let Some(id) = self.selected() else {
            return;
        };
        if self.terminals.contains_key(&id) {
            return;
        }
        let Some(tab) = self.data.tab(id) else {
            return;
        };
        match self.store.history(id) {
            Ok(bytes) => {
                self.terminals.insert(
                    id,
                    Terminal::dormant(
                        tab.rows.clamp(2, 250),
                        tab.cols.clamp(10, 500),
                        tab.cwd.clone(),
                        &bytes,
                    ),
                );
                if self.data.is_remote_tab(id) {
                    if let Some(term) = self.terminals.get(&id) {
                        term.mark_remote(tab.cwd.clone());
                    }
                }
            }
            Err(e) => self.error = Some(format!("无法读取历史：{e:#}")),
        }
    }
    pub fn add_remote_workspace(
        &mut self,
        host_id: Uuid,
        path: String,
        name: String,
    ) -> Result<Uuid> {
        let id = self.data.add_remote_workspace(host_id, path, name)?;
        self.select_workspace(id);
        if self.data.workspace().is_some_and(|w| w.tabs.is_empty()) {
            self.new_tab(crate::ssh::profiles().remove(0));
        } else if let Some(tab) = self.selected() {
            self.restore(tab);
        }
        Ok(id)
    }
    pub fn new_tab(&mut self, profile: CliProfile) {
        let Some(w) = self.data.workspace_mut() else {
            return;
        };
        let tab = Tab::new(profile, w.directory.clone());
        let id = tab.id;
        w.tabs.push(tab);
        w.selected_tab = Some(id);
        self.persist();
        self.start(id);
    }
    fn skip_failed_restore(&mut self, id: Uuid) {
        let saved = {
            let Some(workspace) = self
                .data
                .workspaces
                .iter()
                .find(|workspace| workspace.tabs.iter().any(|tab| tab.id == id))
            else {
                return;
            };
            let Some(tab) = workspace.tabs.iter().find(|tab| tab.id == id) else {
                return;
            };
            self.store.save_failed_restore(workspace, tab)
        };
        if let Err(error) = saved {
            self.error = Some(format!("无法保留恢复失败的会话资料：{error:#}"));
            return;
        }
        self.terminals.remove(&id);
        if let Some(workspace) = self
            .data
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.tabs.iter().any(|tab| tab.id == id))
        {
            let index = workspace
                .tabs
                .iter()
                .position(|tab| tab.id == id)
                .unwrap_or(0);
            workspace.tabs.retain(|tab| tab.id != id);
            if workspace.selected_tab == Some(id) {
                workspace.selected_tab = workspace
                    .tabs
                    .get(index.min(workspace.tabs.len().saturating_sub(1)))
                    .map(|tab| tab.id);
            }
        }
        self.error = None;
        self.persist();
    }
    pub fn restore(&mut self, id: Uuid) {
        self.start_inner(id, true);
    }
    pub fn start(&mut self, id: Uuid) {
        let restoring = self.data.tab(id).is_some_and(|tab| tab.launched);
        self.start_inner(id, restoring);
    }
    fn start_inner(&mut self, id: Uuid, restoring: bool) {
        if self.read_only {
            return;
        }
        self.capture_metadata();
        if self.terminals.get(&id).is_some_and(Terminal::running) {
            return;
        }
        let Some(tab) = self.data.tab(id).cloned() else {
            return;
        };
        if restoring
            && tab.profile.kind == CliKind::Codex
            && tab
                .session_id
                .as_ref()
                .is_none_or(|id| id.trim().is_empty())
        {
            self.skip_failed_restore(id);
            return;
        }
        let result = if let Some(project) = self.data.remote_for_tab(id) {
            match self.data.ssh_hosts.iter().find(|h| h.id == project.host_id) {
                Some(host) => Terminal::start_ssh(
                    &tab,
                    &self.store.root.join("sessions"),
                    host,
                    project,
                    self.grid.0,
                    self.grid.1,
                    self.wake.clone(),
                ),
                None => Err(anyhow::anyhow!(
                    "该工作区的 SSH 主机已不存在，请在设置中检查主机配置"
                )),
            }
        } else {
            Terminal::start(
                &tab,
                &self.store.root.join("sessions"),
                self.grid.0,
                self.grid.1,
                self.wake.clone(),
            )
        };
        match result {
            Ok(term) => {
                if let Some(mut old) = self.terminals.remove(&id) {
                    old.stop();
                    self.archive(id, &old);
                }
                self.terminals.insert(id, term);
                if let Some(tab) = self.data.tab_mut(id) {
                    tab.launched = true;
                }
                self.error = None;
                self.persist();
            }
            Err(_) if restoring => self.skip_failed_restore(id),
            Err(e) => {
                self.error = Some(format!("{e:#}"));
                self.ensure_history();
            }
        }
    }
    pub fn restart(&mut self, id: Uuid) {
        if let Some(mut t) = self.terminals.remove(&id) {
            t.stop();
            self.archive(id, &t);
        }
        if let Some(tab) = self.data.tab_mut(id) {
            tab.launched = false;
            tab.session_summary = None;
            tab.conversation_generation = tab.conversation_generation.wrapping_add(1);
            tab.session_id =
                (tab.profile.kind == CliKind::Claude).then(|| Uuid::new_v4().to_string());
        }
        let _ = std::fs::remove_file(
            self.store
                .root
                .join("sessions")
                .join(format!("{id}.codex.json")),
        );
        // Pi session files also belong to a particular conversation. Keep the old file as an archive.
        let path = self.store.root.join("sessions").join(format!("{id}.jsonl"));
        if path.exists() {
            let _ = std::fs::rename(
                &path,
                path.with_extension(format!("{}.jsonl", Uuid::new_v4())),
            );
        }
        self.start(id);
    }
    pub fn close_tab(&mut self, id: Uuid) {
        self.terminals.remove(&id);
        if let Some(w) = self
            .data
            .workspaces
            .iter_mut()
            .find(|w| w.tabs.iter().any(|t| t.id == id))
        {
            let index = w.tabs.iter().position(|t| t.id == id).unwrap_or(0);
            w.tabs.retain(|t| t.id != id);
            if w.selected_tab == Some(id) {
                w.selected_tab = w
                    .tabs
                    .get(index.min(w.tabs.len().saturating_sub(1)))
                    .map(|t| t.id);
            }
        }
        if !self.read_only {
            self.store.delete_history(id);
        }
        self.ensure_history();
        self.persist();
    }
    pub fn remove_workspace(&mut self, id: Uuid) {
        let ids = self
            .data
            .workspaces
            .iter()
            .find(|w| w.id == id)
            .map(|w| w.tabs.iter().map(|t| t.id).collect::<Vec<_>>())
            .unwrap_or_default();
        for id in ids {
            self.terminals.remove(&id);
            if !self.read_only {
                self.store.delete_history(id);
            }
        }
        self.data.workspaces.retain(|w| w.id != id);
        if self.data.selected_workspace == Some(id) {
            self.data.selected_workspace = self.data.workspaces.first().map(|w| w.id);
        }
        self.ensure_history();
        self.persist();
    }
    pub fn note_codex_submission(&self, id: Uuid) {
        if self.data.is_remote_tab(id)
            || !self
                .data
                .tab(id)
                .is_some_and(|tab| tab.profile.kind == CliKind::Codex)
        {
            return;
        }
        if let Some(terminal) = self
            .terminals
            .get(&id)
            .filter(|terminal| terminal.running())
            && let Ok(mut buffer) = terminal.buffer.lock()
        {
            buffer.parser.callbacks_mut().activity.submitted();
        }
    }
    pub fn send(&mut self, bytes: Vec<u8>) {
        self.send_input(bytes, false);
    }
    pub fn send_input(&mut self, bytes: Vec<u8>, has_image: bool) {
        let Some(id) = self.selected() else {
            return;
        };
        if let Some(t) = self.terminals.get(&id) {
            let submit = bytes == b"\r";
            let interrupt = bytes == b"\x1b" || bytes == b"\x03";
            let codex = (submit || interrupt)
                && !self.data.is_remote_tab(id)
                && self
                    .data
                    .tab(id)
                    .is_some_and(|tab| tab.profile.kind == CliKind::Codex);
            let mut starts_turn = false;
            if let Ok(mut b) = t.buffer.lock() {
                b.parser.screen_mut().set_scrollback(0);
                if codex && submit {
                    starts_turn = has_image
                        || b.parser
                            .screen()
                            .rows(0, b.parser.screen().size().1)
                            .take(b.parser.screen().cursor_position().0 as usize + 1)
                            .collect::<Vec<_>>()
                            .iter()
                            .rev()
                            .take(30)
                            .find_map(|line| {
                                let line = line.trim_start();
                                line.strip_prefix('›')
                                    .or_else(|| line.strip_prefix('❯'))
                                    .map(|text| {
                                        !text.trim().is_empty()
                                            && !text.trim_start().starts_with('/')
                                    })
                            })
                            .unwrap_or(false);
                }
            }
            let result = t.send(bytes);
            if result.is_ok()
                && codex
                && let Ok(mut buffer) = t.buffer.lock()
            {
                if starts_turn {
                    buffer.parser.callbacks_mut().activity.submitted();
                } else if interrupt {
                    buffer.parser.callbacks_mut().activity.interrupted();
                }
            }
            self.result(result);
        }
    }
    pub fn paste(&mut self, text: &str) {
        if let Some(t) = self.selected().and_then(|id| self.terminals.get(&id)) {
            let r = t.paste(text);
            self.result(r);
        }
    }
    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.grid = (rows, cols);
        if let Some(t) = self.selected().and_then(|id| self.terminals.get_mut(&id)) {
            let r = t.resize(rows, cols);
            self.result(r);
        }
    }
    pub fn running_count(&self) -> usize {
        self.terminals.values().filter(|t| t.running()).count()
    }
    pub fn take_dirty(&self) -> bool {
        self.terminals.values().fold(false, |dirty, t| {
            t.dirty.swap(false, Ordering::AcqRel) || dirty
        })
    }
    pub fn capture_metadata(&mut self) {
        for t in self.terminals.values() {
            t.update_cwd();
        }
        for tab in self
            .data
            .workspaces
            .iter_mut()
            .filter(|w| w.remote.is_none())
            .flat_map(|w| &mut w.tabs)
        {
            if tab.profile.kind != CliKind::Codex {
                continue;
            }
            let path = self
                .store
                .root
                .join("sessions")
                .join(format!("{}.codex.json", tab.id));
            if std::fs::metadata(&path).is_ok_and(|m| m.len() <= 16384) {
                if let Ok(bytes) = std::fs::read(&path) {
                    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                        if let Some(id) = value
                            .get("thread-id")
                            .or_else(|| value.get("thread_id"))
                            .and_then(|v| v.as_str())
                        {
                            if Uuid::parse_str(id).is_ok() {
                                tab.session_id = Some(id.into());
                            }
                        }
                    }
                }
            }
        }
    }
    fn archive(&mut self, id: Uuid, t: &Terminal) {
        if self.read_only || !t.loaded() {
            return;
        }
        if let Ok(mut b) = t.buffer.lock() {
            let bytes = b.snapshot();
            let (rows, cols) = b.parser.screen().size();
            let cwd = b.parser.callbacks().cwd.clone();
            if let Some(tab) = self.data.tab_mut(id) {
                tab.rows = rows;
                tab.cols = cols;
                tab.cwd = cwd;
            }
            let result = self.store.save_history(id, &bytes);
            self.result(result);
        }
    }
    pub fn checkpoint(&mut self) {
        if self.read_only {
            return;
        }
        let ids: Vec<_> = self.terminals.keys().copied().collect();
        for id in ids {
            if let Some(t) = self.terminals.remove(&id) {
                self.archive(id, &t);
                self.terminals.insert(id, t);
            }
        }
        self.capture_metadata();
        self.persist();
    }
    pub fn shutdown(&mut self) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        for t in self.terminals.values_mut() {
            t.stop();
        }
        self.checkpoint();
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.shutdown();
    }
}
