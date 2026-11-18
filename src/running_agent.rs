//! Identify an interactive Agent inside a local shell without inspecting terminal text.
use crate::{agents::KNOWN, model::CliProfile};

#[derive(Clone, Debug)]
pub struct RunningAgent {
    pub pid: u32,
    pub profile: CliProfile,
}

fn identify(executable: &str, command_line: &str) -> Option<CliProfile> {
    let executable = executable.to_ascii_lowercase();
    let stem = executable.strip_suffix(".exe").unwrap_or(&executable);
    let script = if matches!(stem, "node" | "bun" | "deno") {
        // Only the entry point identifies an Agent. Prompt arguments can contain
        // arbitrary paths and names, and must never change the tab identity.
        let tokens = command_line
            .split('"')
            .enumerate()
            .flat_map(|(i, part)| {
                if i % 2 == 1 {
                    vec![part]
                } else {
                    part.split_whitespace().collect()
                }
            })
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        tokens
            .get(1)
            .copied()
            .unwrap_or("")
            .replace('\\', "/")
            .to_ascii_lowercase()
    } else {
        String::new()
    };
    let spec = KNOWN.iter().find(|spec| {
        stem == spec.command
            || match spec.id {
                "pi" => {
                    script.contains("/@mariozechner/pi-coding-agent/")
                        || script.contains("/@earendil-works/pi-coding-agent/")
                }
                "codex" => script.ends_with("/@openai/codex/bin/codex.js"),
                "claude" => script.ends_with("/@anthropic-ai/claude-code/cli.js"),
                _ => false,
            }
    })?;
    Some(CliProfile {
        id: spec.id.into(),
        name: spec.name.into(),
        command: spec.command.into(),
        args: Vec::new(),
        kind: spec.kind.clone(),
    })
}

/// One process snapshot for all tabs. Run on a background worker.
pub fn detect(roots: &[u32]) -> Vec<(u32, Option<RunningAgent>)> {
    #[cfg(windows)]
    {
        windows::detect(roots)
    }
    #[cfg(not(windows))]
    {
        roots.iter().map(|pid| (*pid, None)).collect()
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::{collections::HashMap, ffi::c_void, mem::size_of};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
                TH32CS_SNAPPROCESS,
            },
            Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
        },
    };
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtQueryInformationProcess(
            process: HANDLE,
            class: u32,
            info: *mut c_void,
            size: u32,
            returned: *mut u32,
        ) -> i32;
    }
    #[repr(C)]
    struct UnicodeString {
        length: u16,
        maximum_length: u16,
        buffer: *const u16,
    }
    fn command_line(pid: u32) -> Option<String> {
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return None;
            }
            let mut required = 0;
            NtQueryInformationProcess(handle, 60, std::ptr::null_mut(), 0, &mut required);
            if required == 0 || required > 1024 * 1024 {
                CloseHandle(handle);
                return None;
            }
            let mut bytes = vec![0usize; (required as usize).div_ceil(size_of::<usize>())];
            let status = NtQueryInformationProcess(
                handle,
                60,
                bytes.as_mut_ptr().cast(),
                required,
                &mut required,
            );
            CloseHandle(handle);
            if status < 0 {
                return None;
            }
            let value = &*bytes.as_ptr().cast::<UnicodeString>();
            let start = value.buffer as usize;
            let end = start.checked_add(value.length as usize)?;
            let base = bytes.as_ptr() as usize;
            if start < base
                || end > base + bytes.len() * size_of::<usize>()
                || value.length % 2 != 0
            {
                return None;
            }
            Some(String::from_utf16_lossy(std::slice::from_raw_parts(
                value.buffer,
                value.length as usize / 2,
            )))
        }
    }
    pub fn detect(roots: &[u32]) -> Vec<(u32, Option<RunningAgent>)> {
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            // An unavailable snapshot is not evidence that the Agent exited.
            if snapshot == INVALID_HANDLE_VALUE {
                return Vec::new();
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
            let mut processes = HashMap::new();
            let mut ok = Process32FirstW(snapshot, &mut entry);
            while ok != 0 {
                let length = entry
                    .szExeFile
                    .iter()
                    .position(|c| *c == 0)
                    .unwrap_or(entry.szExeFile.len());
                processes.insert(
                    entry.th32ProcessID,
                    (
                        entry.th32ParentProcessID,
                        String::from_utf16_lossy(&entry.szExeFile[..length]),
                    ),
                );
                ok = Process32NextW(snapshot, &mut entry);
            }
            CloseHandle(snapshot);
            roots
                .iter()
                .map(|root| {
                    let mut found: Option<(usize, RunningAgent)> = None;
                    for (&pid, (_, exe)) in &processes {
                        let mut ancestor = pid;
                        let mut depth = 0;
                        while ancestor != *root && depth < 64 {
                            let Some((parent, _)) = processes.get(&ancestor) else {
                                break;
                            };
                            ancestor = *parent;
                            depth += 1;
                        }
                        if ancestor != *root || depth == 0 {
                            continue;
                        }
                        // Avoid opening every unrelated process on the machine.
                        let line = if matches!(
                            exe.to_ascii_lowercase().as_str(),
                            "node.exe" | "bun.exe" | "deno.exe"
                        ) {
                            command_line(pid).unwrap_or_default()
                        } else {
                            String::new()
                        };
                        if let Some(profile) = identify(exe, &line)
                            && found.as_ref().is_none_or(|(previous, _)| depth < *previous)
                        {
                            found = Some((depth, RunningAgent { pid, profile }));
                        }
                    }
                    (*root, found.map(|(_, agent)| agent))
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recognizes_entry_points_without_matching_prompt_text() {
        assert_eq!(identify("codex.exe", "").unwrap().id, "codex");
        assert_eq!(identify("node.exe", r#""C:\Program Files\nodejs\node.exe" "C:\Users\me\node_modules\@earendil-works\pi-coding-agent\dist\cli.js""#).unwrap().id, "pi");
        assert_eq!(
            identify(
                "node.exe",
                r#"node C:\npm\node_modules\@openai\codex\bin\codex.js"#
            )
            .unwrap()
            .id,
            "codex"
        );
        assert!(identify("cmd.exe", "cmd /c echo pi codex").is_none());
        assert!(identify("node.exe", "node server.js @openai/codex/bin/codex.js").is_none());
    }
}
