use std::collections::HashMap;
use std::error::Error;
use std::io::{self, BufRead, Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

// Configurations
const DOC_ID: &str = "1RIr-n3WgdfOYiXgd7J8hk3NcPuIIZhNosTxYcKnEqwI";
const POLL_INTERVAL_SECS: u64 = 5;
const WORKSPACE: &str = "/Users/ze/geminihackathon";

// ANSI escape sequences
const C_RESET: &str = "\x1b[0m";
const C_BOLD: &str = "\x1b[1m";
const C_CYAN: &str = "\x1b[36m";
const C_GREEN: &str = "\x1b[32m";
const C_YELLOW: &str = "\x1b[33m";
const C_MAGENTA: &str = "\x1b[35m";
const C_RED: &str = "\x1b[31m";
const C_BLUE: &str = "\x1b[34m";
const C_GRAY: &str = "\x1b[90m";

#[derive(Clone, Debug)]
struct Tab {
    id: String,
    title: String,
    status: String,
}

#[derive(Clone, Debug)]
struct RunningCommand {
    tab: String,
    command: String,
    start_time: String,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum View {
    Logs,
    About,
    TabOutput,
}

struct AppState {
    doc_title: String,
    last_polled: String,
    tabs_data: Vec<Tab>,
    running_commands: Vec<RunningCommand>,
    logs: Vec<String>,
    tab_outputs: HashMap<String, Vec<String>>,
    running: bool,
    
    selected_selectable_idx: usize,
    current_view: View,
    selected_tab_id: Option<String>,
}

impl AppState {
    fn new() -> Self {
        Self {
            doc_title: "Loading...".to_string(),
            last_polled: "Never".to_string(),
            tabs_data: Vec::new(),
            running_commands: Vec::new(),
            logs: Vec::new(),
            tab_outputs: HashMap::new(),
            running: true,
            selected_selectable_idx: 0,
            current_view: View::Logs,
            selected_tab_id: None,
        }
    }
}

fn strip_ansi(s: &str) -> String {
    let mut result = String::new();
    let mut in_ansi = false;
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\x1b' {
            in_ansi = true;
            i += 1;
            continue;
        }
        if in_ansi {
            if chars[i].is_ascii_alphabetic() {
                in_ansi = false;
            }
            i += 1;
            continue;
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}

fn pad_colored(s: &str, width: usize, fillchar: char, align: &str) -> String {
    let plain_len = strip_ansi(s).chars().count();
    let padding = if width > plain_len { width - plain_len } else { 0 };
    if align == "left" {
        format!("{}{}", s, fillchar.to_string().repeat(padding))
    } else if align == "right" {
        format!("{}{}", fillchar.to_string().repeat(padding), s)
    } else {
        let left_pad = padding / 2;
        let right_pad = padding - left_pad;
        format!(
            "{}{}{}",
            fillchar.to_string().repeat(left_pad),
            s,
            fillchar.to_string().repeat(right_pad)
        )
    }
}

fn add_log(state_mutex: &Arc<Mutex<AppState>>, msg: String) {
    let now_str = get_current_time_str();
    let mut state = state_mutex.lock().unwrap();
    
    // Prevent long log lines from wrapping and breaking UI layout
    let mut display_msg = msg;
    if display_msg.chars().count() > 70 {
        display_msg = display_msg.chars().take(67).collect::<String>() + "...";
    }
    
    state.logs.push(format!("[{}] {}", now_str, display_msg));
    if state.logs.len() > 15 {
        state.logs.remove(0);
    }
}

fn get_current_time_str() -> String {
    if let Ok(output) = Command::new("date").arg("+%H:%M:%S").output() {
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    } else {
        "??:??:??".to_string()
    }
}

fn get_current_datetime_str() -> String {
    if let Ok(output) = Command::new("date").arg("+%Y-%m-%d %H:%M:%S").output() {
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    } else {
        "????-??-?? ??:??:??".to_string()
    }
}

fn get_access_token() -> Option<String> {
    let output = Command::new("gcloud")
        .args(["auth", "print-access-token"])
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

fn fetch_document(token: &str) -> Option<String> {
    let url = format!(
        "https://docs.googleapis.com/v1/documents/{}?includeTabsContent=true",
        DOC_ID
    );
    let output = Command::new("curl")
        .args(["-s", "-H", &format!("Authorization: Bearer {}", token), &url])
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        None
    }
}

fn update_tab_title_api(tab_id: &str, new_title: &str, token: &str) -> Result<(), Box<dyn Error>> {
    let url = format!("https://docs.googleapis.com/v1/documents/{}:batchUpdate", DOC_ID);
    
    let body = format!(
        r#"{{"requests": [{{"updateDocumentTabProperties": {{"tabProperties": {{"tabId": "{}", "title": "{}"}}, "fields": "title"}}}}]}}"#,
        tab_id, new_title
    );
    
    let output = Command::new("curl")
        .args([
            "-s",
            "-X", "POST",
            "-H", &format!("Authorization: Bearer {}", token),
            "-H", "Content-Type: application/json",
            "-d", &body,
            &url
        ])
        .output()?;
        
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string().into())
    }
}

fn extract_doc_title(json: &str) -> String {
    if let Some(title_start) = json.find("\"title\"") {
        let sub = &json[title_start..];
        if let Some(col_idx) = sub.find(':') {
            let after_col = &sub[col_idx..];
            if let Some(q1) = after_col.find('"') {
                let val_start = &after_col[q1+1..];
                if let Some(q2) = val_start.find('"') {
                    return val_start[..q2].to_string();
                }
            }
        }
    }
    "Unknown".to_string()
}

fn extract_tabs(json: &str) -> Vec<Tab> {
    let mut tabs = Vec::new();
    let mut search_idx = 0;
    
    while let Some(prop_idx) = json[search_idx..].find("\"tabProperties\"") {
        let absolute_prop_idx = search_idx + prop_idx;
        let next_start = absolute_prop_idx + "\"tabProperties\"".len();
        
        let next_prop_find = json[next_start..].find("\"tabProperties\"");
        let limit = match next_prop_find {
            Some(idx) => next_start + idx,
            None => json.len(),
        };
        
        let sub = &json[next_start..limit];
        
        let mut tab_id = String::new();
        if let Some(id_pos) = sub.find("\"tabId\"") {
            let after_id = &sub[id_pos..];
            if let Some(col_pos) = after_id.find(':') {
                let after_col = &after_id[col_pos..];
                if let Some(q1) = after_col.find('"') {
                    let after_q1 = &after_col[q1+1..];
                    if let Some(q2) = after_q1.find('"') {
                        tab_id = after_q1[..q2].to_string();
                    }
                }
            }
        }
        
        let mut title = String::new();
        if let Some(title_pos) = sub.find("\"title\"") {
            let after_title = &sub[title_pos..];
            if let Some(col_pos) = after_title.find(':') {
                let after_col = &after_title[col_pos..];
                if let Some(q1) = after_col.find('"') {
                    let after_q1 = &after_col[q1+1..];
                    if let Some(q2) = after_q1.find('"') {
                        title = after_q1[..q2].to_string();
                    }
                }
            }
        }
        
        if !tab_id.is_empty() && !title.is_empty() {
            let status = if title.contains("[agent,working]") {
                "WORKING".to_string()
            } else if title.contains("[agent]") {
                "READY".to_string()
            } else if title.contains("[human]") {
                "HUMAN".to_string()
            } else {
                "Idle".to_string()
            };
            
            if !tabs.iter().any(|t: &Tab| t.id == tab_id) {
                tabs.push(Tab {
                    id: tab_id,
                    title,
                    status,
                });
            }
        }
        
        search_idx = next_start;
    }
    tabs
}

fn run_agent_command(tab_title: String, tab_id: String, state_mutex: Arc<Mutex<AppState>>) {
    let doc_link = format!(
        "https://docs.google.com/document/d/{}/edit?tab=t.a6bge8wm2nyh",
        DOC_ID
    );
    let prompt = format!(
        "read {} of the {} via API, and update the contents in the tab accordingly then update [agent,working] in the title to [human]",
        tab_title, doc_link
    );
    
    let cmd_args = vec![
        "--dangerously-skip-permissions".to_string(),
        "--print".to_string(),
        prompt,
    ];
    
    let cmd_str = format!("agy {}", cmd_args.join(" "));
    let start_time = get_current_time_str();
    
    let cmd_info = RunningCommand {
        tab: tab_title.clone(),
        command: cmd_str,
        start_time,
    };
    
    {
        let mut state = state_mutex.lock().unwrap();
        state.running_commands.push(cmd_info.clone());
    }
    
    add_log(&state_mutex, format!("Starting agy for tab: '{}'", tab_title));
    
    let child_res = Command::new("agy")
        .args(&cmd_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .current_dir(WORKSPACE)
        .spawn();
        
    match child_res {
        Ok(mut child) => {
            let stdout = child.stdout.take().unwrap();
            let stderr = child.stderr.take().unwrap();
            
            let state_clone_1 = Arc::clone(&state_mutex);
            let tab_id_clone_1 = tab_id.clone();
            let stdout_thread = thread::spawn(move || {
                let reader = io::BufReader::new(stdout);
                for line in reader.lines() {
                    if let Ok(l) = line {
                        let trimmed = l.trim().to_string();
                        if !trimmed.is_empty() {
                            add_log(&state_clone_1, format!(" agy: {}", trimmed));
                            let mut state = state_clone_1.lock().unwrap();
                            state.tab_outputs.entry(tab_id_clone_1.clone())
                                .or_insert_with(Vec::new)
                                .push(trimmed);
                        }
                    }
                }
            });
            
            let state_clone_2 = Arc::clone(&state_mutex);
            let tab_id_clone_2 = tab_id.clone();
            let stderr_thread = thread::spawn(move || {
                let reader = io::BufReader::new(stderr);
                for line in reader.lines() {
                    if let Ok(l) = line {
                        let trimmed = l.trim().to_string();
                        if !trimmed.is_empty() {
                            add_log(&state_clone_2, format!(" agy err: {}", trimmed));
                            let mut state = state_clone_2.lock().unwrap();
                            state.tab_outputs.entry(tab_id_clone_2.clone())
                                .or_insert_with(Vec::new)
                                .push(trimmed);
                        }
                    }
                }
            });
            
            let status = child.wait();
            let _ = stdout_thread.join();
            let _ = stderr_thread.join();
            
            match status {
                Ok(exit_status) => {
                    if exit_status.success() {
                        add_log(&state_mutex, format!("agy finished successfully for tab: '{}'", tab_title));
                    } else {
                        add_log(&state_mutex, format!("agy error code {} for tab: '{}'", exit_status.code().unwrap_or(-1), tab_title));
                    }
                }
                Err(e) => {
                    add_log(&state_mutex, format!("Error waiting for agy: {}", e));
                }
            }
        }
        Err(e) => {
            add_log(&state_mutex, format!("Error spawning agy: {}", e));
        }
    }
    
    {
        let mut state = state_mutex.lock().unwrap();
        state.running_commands.retain(|c| c.tab != tab_title);
    }
}

fn poll_loop(state_mutex: Arc<Mutex<AppState>>) {
    while state_mutex.lock().unwrap().running {
        if let Some(token) = get_access_token() {
            if let Some(doc_json) = fetch_document(&token) {
                let doc_title = extract_doc_title(&doc_json);
                let current_tabs = extract_tabs(&doc_json);
                let now_str = get_current_datetime_str();
                
                {
                    let mut state = state_mutex.lock().unwrap();
                    state.doc_title = doc_title;
                    state.last_polled = now_str;
                    state.tabs_data = current_tabs.clone();
                }
                
                for tab in &current_tabs {
                    if tab.status == "READY" {
                        let tab_id = tab.id.clone();
                        let old_title = tab.title.clone();
                        let new_title = old_title.replace("[agent]", "[agent,working]");
                        
                        add_log(&state_mutex, format!("Renaming tab '{}' -> '{}'...", old_title, new_title));
                        
                        match update_tab_title_api(&tab_id, &new_title, &token) {
                            Ok(_) => {
                                add_log(&state_mutex, format!("Tab renamed to '{}'", new_title));
                                
                                let state_clone = Arc::clone(&state_mutex);
                                let tab_title_clone = new_title.clone();
                                let tab_id_clone = tab_id.clone();
                                thread::spawn(move || {
                                    run_agent_command(tab_title_clone, tab_id_clone, state_clone);
                                });
                            }
                            Err(e) => {
                                add_log(&state_mutex, format!("Rename Error: {}", e));
                            }
                        }
                    }
                }
            } else {
                add_log(&state_mutex, "Poll Error: Failed to fetch document".to_string());
            }
        } else {
            add_log(&state_mutex, "Token Error: Failed to print access token".to_string());
        }
        
        for _ in 0..(POLL_INTERVAL_SECS * 2) {
            if !state_mutex.lock().unwrap().running {
                break;
            }
            thread::sleep(Duration::from_millis(500));
        }
    }
}

fn draw_ui(state_mutex: &Arc<Mutex<AppState>>) {
    let state = state_mutex.lock().unwrap();
    
    // Clear screen and move cursor home
    print!("\x1b[2J\x1b[H");
    
    let mut left_lines = Vec::new();
    
    // Section 1: Mini Header Banner
    left_lines.push(format!("{}{}┌──────────────────────────────┐{}", C_BOLD, C_CYAN, C_RESET));
    left_lines.push(format!("{}{}│     ORCHESTRATOR PANEL       │{}", C_BOLD, C_CYAN, C_RESET));
    left_lines.push(format!("{}{}└──────────────────────────────┘{}", C_BOLD, C_CYAN, C_RESET));
    left_lines.push("".to_string());
    
    // Section 2: Document Info Card
    left_lines.push(format!("{}{}Document Info:{}", C_CYAN, C_BOLD, C_RESET));
    let mut trunc_title = state.doc_title.clone();
    if trunc_title.chars().count() > 20 {
        trunc_title = trunc_title.chars().take(17).collect::<String>() + "...";
    }
    left_lines.push(format!(" • Title   : {}{}{}", C_BOLD, trunc_title, C_RESET));
    let doc_id_short = if DOC_ID.len() > 12 { &DOC_ID[..12] } else { DOC_ID };
    left_lines.push(format!(" • ID      : {}{}...{}", C_BLUE, doc_id_short, C_RESET));
    
    let mut short_polled = state.last_polled.clone();
    if short_polled.contains(' ') {
        if let Some(last_part) = short_polled.split(' ').last() {
            short_polled = last_part.to_string();
        }
    }
    left_lines.push(format!(" • Updated : {}{}{}", C_GRAY, short_polled, C_RESET));
    left_lines.push(format!(" • Interval: {}s", POLL_INTERVAL_SECS));
    left_lines.push("".to_string());
    
    // Section 3: Navigation Selectables
    struct Selectable {
        item_type: &'static str,
        label: String,
        id: String,
    }
    let mut selectables = vec![
        Selectable { item_type: "about", label: "About".to_string(), id: "about".to_string() },
        Selectable { item_type: "logs", label: "Global Activity Logs".to_string(), id: "logs".to_string() },
    ];
    for tab in &state.tabs_data {
        selectables.push(Selectable {
            item_type: "tab",
            label: format!("Tab: {}", tab.title),
            id: tab.id.clone(),
        });
    }
    
    left_lines.push(format!("{}{}Navigation Menu:{}", C_CYAN, C_BOLD, C_RESET));
    for (idx, item) in selectables.iter().enumerate() {
        let label_trunc = if item.label.chars().count() > 24 {
            item.label.chars().take(21).collect::<String>() + "..."
        } else {
            item.label.clone()
        };
        if idx == state.selected_selectable_idx {
            left_lines.push(format!(" {}▶{} {}{}{}", C_GREEN, C_RESET, C_BOLD, C_GREEN, label_trunc));
        } else {
            left_lines.push(format!("   {}", label_trunc));
        }
    }
    left_lines.push("".to_string());
    
    // Section 4: Running Commands Card
    left_lines.push(format!("{}{}Running Commands:{}", C_CYAN, C_BOLD, C_RESET));
    if !state.running_commands.is_empty() {
        for cmd_item in &state.running_commands {
            let mut tab_name = cmd_item.tab.clone();
            if tab_name.chars().count() > 18 {
                tab_name = tab_name.chars().take(15).collect::<String>() + "...";
            }
            left_lines.push(format!(" • {} {}{}{}", cmd_item.start_time, C_YELLOW, tab_name, C_RESET));
        }
    } else {
        left_lines.push(format!(" {}(No active commands){}", C_GRAY, C_RESET));
    }
    
    let mut right_lines = Vec::new();
    
    match state.current_view {
        View::Logs => {
            // Section 1: Tabs Table
            right_lines.push(format!("{}{}Document Tabs Status:{}", C_CYAN, C_BOLD, C_RESET));
            right_lines.push(format!("  {}{:<10} {:<26} {:<12}{}", C_BOLD, "Tab ID", "Tab Title", "Status", C_RESET));
            for tab in &state.tabs_data {
                let mut tid = tab.id.clone();
                if tid.len() > 10 {
                    tid = tid[..8].to_string() + "..";
                }
                let mut title = tab.title.clone();
                if title.chars().count() > 26 {
                    title = title.chars().take(23).collect::<String>() + "...";
                }
                let status_str = match tab.status.as_str() {
                    "READY" => format!("{}{:<12}{}", C_MAGENTA, tab.status, C_RESET),
                    "WORKING" => format!("{}{:<12}{}", C_YELLOW, tab.status, C_RESET),
                    "HUMAN" => format!("{}{:<12}{}", C_BLUE, tab.status, C_RESET),
                    _ => format!("{}{:<12}{}", C_GREEN, tab.status, C_RESET),
                };
                right_lines.push(format!("  {:<10} {:<26} {}", tid, title, status_str));
            }
            right_lines.push("".to_string());
            
            // Section 2: Activity Logs Box
            right_lines.push(format!("{}{}Global Activity Logs:{}", C_CYAN, C_BOLD, C_RESET));
            if !state.logs.is_empty() {
                for log in &state.logs {
                    right_lines.push(format!("  {}", log));
                }
            } else {
                right_lines.push("  (No logs yet. Initializing...)".to_string());
            }
        }
        View::About => {
            right_lines.push(format!("{}{}About Orchestrator:{}", C_CYAN, C_BOLD, C_RESET));
            right_lines.push("".to_string());
            right_lines.push("  Google Docs Agent Orchestrator TUI (Rust Edition)".to_string());
            right_lines.push("".to_string());
            right_lines.push(format!("  {}★ PRIMARY SYSTEM INPUT:{}", C_CYAN, C_RESET));
            right_lines.push("  Each individual Google Doc Tab serves as a direct".to_string());
            right_lines.push("  input/job channel to this orchestration system.".to_string());
            right_lines.push("  Trigger/Input state transitions on Tab Titles:".to_string());
            right_lines.push("   • [agent]         -> System Input / Job Trigger".to_string());
            right_lines.push("   • [agent,working] -> Running Background Process".to_string());
            right_lines.push("   • [human]         -> Task Completed".to_string());
            right_lines.push("".to_string());
            right_lines.push(format!("  {}Active Command Template:{}", C_BOLD, C_RESET));
            right_lines.push(format!("  {}agy --dangerously-skip-permissions --print \\{}", C_YELLOW, C_RESET));
            let doc_id_trunc = if DOC_ID.len() > 15 { &DOC_ID[..15] } else { DOC_ID };
            right_lines.push(format!("    {}\"read <Tab Title> of the {}...\\{}", C_YELLOW, doc_id_trunc, C_RESET));
            right_lines.push(format!("    {}via API, and update the contents in the tab\\{}", C_YELLOW, C_RESET));
            right_lines.push(format!("    {}accordingly then update to [human]\"{}", C_YELLOW, C_RESET));
            right_lines.push("".to_string());
            right_lines.push(format!("  {}Navigation Info:{}", C_BOLD, C_RESET));
            right_lines.push("  • Use arrow keys [↑/↓] or [w/s] or [j/k] to navigate.".to_string());
            right_lines.push("  • Press [Enter] or [Space] on a tab to select it.".to_string());
            right_lines.push("  • Click on navigation menu or table tabs using mouse.".to_string());
            right_lines.push("  • Select \"Global Activity Logs\" to return to logs.".to_string());
        }
        View::TabOutput => {
            let mut tab_name = "Unknown Tab".to_string();
            if let Some(ref sel_id) = state.selected_tab_id {
                for t in &state.tabs_data {
                    if t.id == *sel_id {
                        tab_name = t.title.clone();
                        break;
                    }
                }
            }
            let tab_name_trunc = if tab_name.chars().count() > 24 {
                tab_name.chars().take(21).collect::<String>() + "..."
            } else {
                tab_name
            };
            right_lines.push(format!("{}{}Output of tab: '{}'{}", C_CYAN, C_BOLD, tab_name_trunc, C_RESET));
            right_lines.push(format!("  {}Last execution stream output:{}", C_GRAY, C_RESET));
            right_lines.push("".to_string());
            
            let empty_vec = Vec::new();
            let outputs = state.selected_tab_id.as_ref()
                .and_then(|id| state.tab_outputs.get(id))
                .unwrap_or(&empty_vec);
                
            if !outputs.is_empty() {
                let start_idx = if outputs.len() > 15 { outputs.len() - 15 } else { 0 };
                for out in &outputs[start_idx..] {
                    right_lines.push(format!("  {}▶{} {}", C_GREEN, C_RESET, out));
                }
            } else {
                right_lines.push("  (No output captured for this tab yet.)".to_string());
                right_lines.push("".to_string());
                right_lines.push("  To capture output, set the tab title status tag".to_string());
                right_lines.push("  to [agent] in Google Docs. TUI will trigger a run".to_string());
                right_lines.push("  and capture the stdout/stderr stream here.".to_string());
            }
        }
    }
    let max_lines = left_lines.len().max(right_lines.len());
    for i in 0..max_lines {
        let left = if i < left_lines.len() { &left_lines[i] } else { "" };
        let right = if i < right_lines.len() { &right_lines[i] } else { "" };
        
        let padded_left = pad_colored(left, 32, ' ', "left");
        let padded_right = pad_colored(right, 52, ' ', "left");
        
        print!(" {} {}│{} {}\r\n", padded_left, C_GRAY, C_RESET, padded_right);
    }
    
    print!(" {}{}{}─{}{}\r\n", C_GRAY, "─".repeat(32), "┼", "─".repeat(54), C_RESET);
    
    // Footer Card
    let footer_width = 85;
    let line1 = pad_colored(
        &format!("{}{}Quick Controls:{} [↑/↓] Navigate | [Enter] Select/Click | [q] Exit", C_BOLD, C_RESET, C_RESET),
        footer_width,
        ' ',
        "left"
    );
    let current_view_str = match state.current_view {
        View::Logs => "LOGS",
        View::About => "ABOUT",
        View::TabOutput => "TAB_OUTPUT",
    };
    let line2 = pad_colored(
        &format!("{}{}Current View:{} {}{}{}", C_BOLD, C_RESET, C_RESET, C_GREEN, current_view_str, C_RESET),
        footer_width,
        ' ',
        "left"
    );
    
    print!(" {}┌{}┐{}\r\n", C_GRAY, "─".repeat(footer_width + 2), C_RESET);
    print!(" {}│{} {} │{}\r\n", C_GRAY, C_RESET, line1, C_RESET);
    print!(" {}│{} {} │{}\r\n", C_GRAY, C_RESET, line2, C_RESET);
    print!(" {}└{}┘{}\r\n", C_GRAY, "─".repeat(footer_width + 2), C_RESET);
    
    io::stdout().flush().unwrap();
}

struct MouseClick {
    cb: i32,
    cx: usize,
    cy: usize,
    event: char,
}

fn parse_mouse_click(bytes: &[u8]) -> Option<MouseClick> {
    let s = String::from_utf8_lossy(bytes);
    if s.starts_with("\x1b[<") {
        let trimmed = s.trim_start_matches("\x1b[<");
        let event = s.chars().last()?;
        let content = &trimmed[..trimmed.len() - 1];
        let parts: Vec<&str> = content.split(';').collect();
        if parts.len() == 3 {
            let cb = parts[0].parse::<i32>().ok()?;
            let cx = parts[1].parse::<usize>().ok()?;
            let cy = parts[2].parse::<usize>().ok()?;
            return Some(MouseClick { cb, cx, cy, event });
        }
    }
    None
}

fn handle_input_bytes(bytes: &[u8], state_mutex: &Arc<Mutex<AppState>>) {
    let mut state = state_mutex.lock().unwrap();
    
    struct Selectable {
        item_type: &'static str,
        id: String,
    }
    let mut selectables = vec![
        Selectable { item_type: "about", id: "about".to_string() },
        Selectable { item_type: "logs", id: "logs".to_string() },
    ];
    for tab in &state.tabs_data {
        selectables.push(Selectable {
            item_type: "tab",
            id: tab.id.clone(),
        });
    }
    
    if selectables.is_empty() {
        return;
    }
    
    if bytes.starts_with(b"\x1b[<") {
        if let Some(click) = parse_mouse_click(bytes) {
            if click.event == 'M' && click.cb == 0 { // Left press
                if click.cx <= 34 {
                    if click.cy >= 12 {
                        let idx = click.cy - 12;
                        if idx < selectables.len() {
                            state.selected_selectable_idx = idx;
                            let item = &selectables[idx];
                            if item.item_type == "about" {
                                state.current_view = View::About;
                            } else if item.item_type == "logs" {
                                state.current_view = View::Logs;
                            } else if item.item_type == "tab" {
                                state.current_view = View::TabOutput;
                                state.selected_tab_id = Some(item.id.clone());
                            }
                        }
                    }
                } else if click.cx > 34 && state.current_view == View::Logs {
                    if click.cy >= 3 {
                        let tab_idx = click.cy - 3;
                        if tab_idx < state.tabs_data.len() {
                            let clicked_tab_id = state.tabs_data[tab_idx].id.clone();
                            for (s_idx, s) in selectables.iter().enumerate() {
                                if s.item_type == "tab" && s.id == clicked_tab_id {
                                    state.selected_selectable_idx = s_idx;
                                    state.current_view = View::TabOutput;
                                    state.selected_tab_id = Some(clicked_tab_id.clone());
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
        return;
    }
    
    if bytes == b"\x1b[A" || bytes == b"k" || bytes == b"w" {
        if state.selected_selectable_idx > 0 {
            state.selected_selectable_idx -= 1;
        } else {
            state.selected_selectable_idx = selectables.len() - 1;
        }
    } else if bytes == b"\x1b[B" || bytes == b"j" || bytes == b"s" {
        state.selected_selectable_idx = (state.selected_selectable_idx + 1) % selectables.len();
    } else if bytes == b"\r" || bytes == b"\n" || bytes == b" " {
        let idx = state.selected_selectable_idx.min(selectables.len() - 1);
        let item = &selectables[idx];
        if item.item_type == "about" {
            state.current_view = View::About;
        } else if item.item_type == "logs" {
            state.current_view = View::Logs;
        } else if item.item_type == "tab" {
            state.current_view = View::TabOutput;
            state.selected_tab_id = Some(item.id.clone());
        }
    } else if bytes == b"q" || bytes == b"Q" || bytes == b"\x03" {
        state.running = false;
    }
}

fn spawn_input_thread() -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buffer = [0u8; 1024];
        let mut stdin = io::stdin();
        loop {
            match stdin.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    let bytes = buffer[..n].to_vec();
                    if tx.send(bytes).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    rx
}

fn main() {
    let state = Arc::new(Mutex::new(AppState::new()));
    
    // Set raw mode and disable echo
    let _ = Command::new("stty").args(["raw", "-echo"]).status();
    // Enable mouse reporting and SGR mode
    print!("\x1b[?1000h\x1b[?1006h");
    io::stdout().flush().unwrap();
    
    let rx = spawn_input_thread();
    
    let state_clone = Arc::clone(&state);
    let poll_thread = thread::spawn(move || {
        poll_loop(state_clone);
    });
    
    while state.lock().unwrap().running {
        draw_ui(&state);
        
        if let Ok(bytes) = rx.recv_timeout(Duration::from_millis(100)) {
            handle_input_bytes(&bytes, &state);
        }
    }
    
    // Cleanup: disable mouse reporting, restore term settings
    print!("\x1b[?1000l\x1b[?1006l");
    io::stdout().flush().unwrap();
    let _ = Command::new("stty").arg("sane").status();
    
    // Joint poll thread
    {
        let mut s = state.lock().unwrap();
        s.running = false;
    }
    let _ = poll_thread.join();
    
    println!("\nExiting orchestrator...");
}
