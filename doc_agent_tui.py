import sys
import os
import time
import json
import urllib.request
import urllib.error
import subprocess
import threading
from datetime import datetime
import re
import select
import tty
import termios

# Configurations
DOC_ID = "1RIr-n3WgdfOYiXgd7J8hk3NcPuIIZhNosTxYcKnEqwI"
DOC_LINK = f"https://docs.google.com/document/d/{DOC_ID}/edit?tab=t.a6bge8wm2nyh"
POLL_INTERVAL = 5  # seconds
WORKSPACE = "/Users/ze/geminihackathon"

# Shared State
state_lock = threading.Lock()
doc_title = "Loading..."
last_polled = "Never"
tabs_data = []  # List of dicts: {"id": str, "title": str, "status": str}
running_commands = []  # List of dicts: {"tab": str, "command": str, "start_time": str}
logs = []
running = True

# Interactive State
selected_selectable_idx = 0
current_view = "LOGS"  # "LOGS", "ABOUT", "TAB_OUTPUT"
selected_tab_id = None
tab_outputs = {}  # {tab_id: [log lines]}

# Colors (ANSI escape sequences)
C_RESET = "\033[0m"
C_BOLD = "\033[1m"
C_CYAN = "\033[36m"
C_GREEN = "\033[32m"
C_YELLOW = "\033[33m"
C_MAGENTA = "\033[35m"
C_RED = "\033[31m"
C_BLUE = "\033[34m"
C_GRAY = "\033[90m"

def strip_ansi(s):
    return re.sub(r'\033\[[0-9;]*[a-zA-Z]', '', s)

def pad_colored(s, width, fillchar=" ", align="left"):
    plain_len = len(strip_ansi(s))
    padding = max(0, width - plain_len)
    if align == "left":
        return s + (fillchar * padding)
    elif align == "right":
        return (fillchar * padding) + s
    else:
        left_pad = padding // 2
        right_pad = padding - left_pad
        return (fillchar * left_pad) + s + (fillchar * right_pad)

def add_log(msg):
    timestamp = datetime.now().strftime("%H:%M:%S")
    with state_lock:
        # Prevent very long lines from breaking the TUI layout
        if len(msg) > 70:
            msg = msg[:67] + "..."
        logs.append(f"[{timestamp}] {msg}")
        if len(logs) > 15:
            logs.pop(0)

def draw_ui():
    with state_lock:
        # Clear screen and move cursor home
        sys.stdout.write("\033[2J\033[H")
        
        left_lines = []
        
        # Section 1: Mini Header Banner
        left_lines.append(f"{C_BOLD}{C_CYAN}┌──────────────────────────────┐{C_RESET}")
        left_lines.append(f"{C_BOLD}{C_CYAN}│     ORCHESTRATOR PANEL       │{C_RESET}")
        left_lines.append(f"{C_BOLD}{C_CYAN}└──────────────────────────────┘{C_RESET}")
        left_lines.append("")
        
        # Section 2: Document Info Card
        left_lines.append(f"{C_CYAN}{C_BOLD}Document Info:{C_RESET}")
        trunc_title = doc_title
        if len(trunc_title) > 20:
            trunc_title = trunc_title[:17] + "..."
        left_lines.append(f" • Title   : {C_BOLD}{trunc_title}{C_RESET}")
        left_lines.append(f" • ID      : {C_BLUE}{DOC_ID[:12]}...{C_RESET}")
        short_polled = last_polled
        if len(short_polled) > 10:
            short_polled = short_polled.split(" ")[-1]
        left_lines.append(f" • Updated : {C_GRAY}{short_polled}{C_RESET}")
        left_lines.append(f" • Interval: {POLL_INTERVAL}s")
        left_lines.append("")
        
        # Section 3: Navigation Selectables
        selectables = []
        selectables.append({"type": "about", "label": "About", "id": "about"})
        selectables.append({"type": "logs", "label": "Global Activity Logs", "id": "logs"})
        for tab in tabs_data:
            selectables.append({"type": "tab", "label": f"Tab: {tab['title']}", "id": tab["id"]})
            
        left_lines.append(f"{C_CYAN}{C_BOLD}Navigation Menu:{C_RESET}")
        for idx, item in enumerate(selectables):
            if idx == selected_selectable_idx:
                left_lines.append(f" {C_GREEN}▶{C_RESET} {C_BOLD}{C_GREEN}{item['label'][:24]}{C_RESET}")
            else:
                left_lines.append(f"   {item['label'][:24]}")
        left_lines.append("")
        
        # Section 4: Running Commands Card
        left_lines.append(f"{C_CYAN}{C_BOLD}Running Commands:{C_RESET}")
        if running_commands:
            for cmd_item in running_commands:
                tab_name = cmd_item["tab"]
                if len(tab_name) > 18:
                    tab_name = tab_name[:15] + "..."
                left_lines.append(f" • {cmd_item['start_time']} {C_YELLOW}{tab_name}{C_RESET}")
        else:
            left_lines.append(f" {C_GRAY}(No active commands){C_RESET}")
            
        right_lines = []
        
        if current_view == "LOGS":
            # Section 1: Tabs Table
            right_lines.append(f"{C_CYAN}{C_BOLD}Document Tabs Status:{C_RESET}")
            right_lines.append(f"  {C_BOLD}{'Tab ID':<10} {'Tab Title':<26} {'Status':<12}{C_RESET}")
            for tab in tabs_data:
                tid = tab["id"]
                if len(tid) > 10:
                    tid = tid[:8] + ".."
                title = tab["title"]
                if len(title) > 26:
                    title = title[:23] + "..."
                status = tab["status"]
                
                if status == "READY":
                    status_str = f"{C_MAGENTA}{status:<12}{C_RESET}"
                elif status == "WORKING":
                    status_str = f"{C_YELLOW}{status:<12}{C_RESET}"
                elif status == "HUMAN":
                    status_str = f"{C_BLUE}{status:<12}{C_RESET}"
                else:
                    status_str = f"{C_GREEN}{status:<12}{C_RESET}"
                    
                right_lines.append(f"  {tid:<10} {title:<26} {status_str}")
                
            right_lines.append("")
            
            # Section 2: Activity Logs Box
            right_lines.append(f"{C_CYAN}{C_BOLD}Global Activity Logs:{C_RESET}")
            if logs:
                for log in logs:
                    right_lines.append(f"  {log}")
            else:
                right_lines.append("  (No logs yet. Initializing...)")
                
        elif current_view == "ABOUT":
            right_lines.append(f"{C_CYAN}{C_BOLD}About Orchestrator:{C_RESET}")
            right_lines.append("")
            right_lines.append("  Google Docs Agent Orchestrator TUI v2.2")
            right_lines.append("  Automates and orchestrates Gemini-powered agents")
            right_lines.append("  reading and updating specific document tabs.")
            right_lines.append("")
            right_lines.append(f"  {C_BOLD}Active Command Template:{C_RESET}")
            right_lines.append(f"  {C_YELLOW}agy --dangerously-skip-permissions --print \\{C_RESET}")
            right_lines.append(f"    {C_YELLOW}\"read <Tab Title> of the {DOC_ID[:15]}...\\{C_RESET}")
            right_lines.append(f"    {C_YELLOW}via API, and update the contents in the tab\\{C_RESET}")
            right_lines.append(f"    {C_YELLOW}accordingly then update to [human]\"{C_RESET}")
            right_lines.append("")
            right_lines.append(f"  {C_BOLD}Navigation Info:{C_RESET}")
            right_lines.append("  • Use arrow keys [↑/↓] or [w/s] or [j/k] to navigate.")
            right_lines.append("  • Press [Enter] or [Space] on a tab to select it.")
            right_lines.append("  • Select \"Global Activity Logs\" to return to logs.")
            
        elif current_view == "TAB_OUTPUT":
            tab_name = "Unknown Tab"
            for t in tabs_data:
                if t["id"] == selected_tab_id:
                    tab_name = t["title"]
                    break
                    
            right_lines.append(f"{C_CYAN}{C_BOLD}Output of tab: '{tab_name[:24]}'{C_RESET}")
            right_lines.append(f"  {C_GRAY}Last execution stream output:{C_RESET}")
            right_lines.append("")
            
            outputs = tab_outputs.get(selected_tab_id, [])
            if outputs:
                # Show the last 15 output lines
                for out in outputs[-15:]:
                    right_lines.append(f"  {C_GREEN}▶{C_RESET} {out}")
            else:
                right_lines.append("  (No output captured for this tab yet.)")
                right_lines.append("")
                right_lines.append("  To capture output, set the tab title status tag")
                right_lines.append("  to [agent] in Google Docs. TUI will trigger a run")
                right_lines.append("  and capture the stdout/stderr stream here.")
                
        # Draw Side-by-Side Columns
        max_lines = max(len(left_lines), len(right_lines))
        for i in range(max_lines):
            left = left_lines[i] if i < len(left_lines) else ""
            right = right_lines[i] if i < len(right_lines) else ""
            
            padded_left = pad_colored(left, 32)
            padded_right = pad_colored(right, 52)
            
            sys.stdout.write(f" {padded_left} {C_GRAY}│{C_RESET} {padded_right}\n")
            
        sys.stdout.write(f" {C_GRAY}" + "─" * 32 + "┼" + "─" * 54 + f"{C_RESET}\n")
        
        # Footer Card
        footer_width = 85
        line1 = pad_colored(f"{C_BOLD}Quick Controls:{C_RESET} [↑/↓] Navigate | [Enter] Select/Click | [q] Exit", footer_width)
        line2 = pad_colored(f"{C_BOLD}Current View:{C_RESET} {C_GREEN}{current_view}{C_RESET}", footer_width)
        
        sys.stdout.write(f" {C_GRAY}┌" + "─" * (footer_width + 2) + f"┐{C_RESET}\n")
        sys.stdout.write(f" {C_GRAY}│{C_RESET} {line1} {C_GRAY}│{C_RESET}\n")
        sys.stdout.write(f" {C_GRAY}│{C_RESET} {line2} {C_GRAY}│{C_RESET}\n")
        sys.stdout.write(f" {C_GRAY}└" + "─" * (footer_width + 2) + f"┘{C_RESET}\n")
        sys.stdout.flush()

def get_access_token():
    try:
        res = subprocess.run(["gcloud", "auth", "print-access-token"], capture_output=True, text=True, check=True)
        return res.stdout.strip()
    except Exception as e:
        add_log(f"Token Error: {str(e)}")
        return None

def fetch_document(token):
    url = f"https://docs.googleapis.com/v1/documents/{DOC_ID}?includeTabsContent=true"
    req = urllib.request.Request(url)
    req.add_header("Authorization", f"Bearer {token}")
    with urllib.request.urlopen(req) as response:
        return json.loads(response.read().decode('utf-8'))

def update_tab_title(tab_id, new_title, token):
    url = f"https://docs.googleapis.com/v1/documents/{DOC_ID}:batchUpdate"
    req = urllib.request.Request(url, method="POST")
    req.add_header("Authorization", f"Bearer {token}")
    req.add_header("Content-Type", "application/json")
    
    body = {
        "requests": [
            {
                "updateDocumentTabProperties": {
                    "tabProperties": {
                        "tabId": tab_id,
                        "title": new_title
                    },
                    "fields": "title"
                }
            }
        ]
    }
    
    req_data = json.dumps(body).encode('utf-8')
    with urllib.request.urlopen(req, data=req_data) as response:
        return json.loads(response.read().decode('utf-8'))

def run_agent_command(tab_title, tab_id):
    add_log(f"Starting agy for tab: '{tab_title}'")
    prompt = f"read {tab_title} of the {DOC_LINK} via API, and update the contents in the tab accordingly then update [agent,working] in the title to [human]"
    cmd = ["agy", "--dangerously-skip-permissions", "--print", prompt]
    cmd_str = " ".join(cmd)
    
    start_time = datetime.now().strftime("%H:%M:%S")
    cmd_info = {"tab": tab_title, "command": cmd_str, "start_time": start_time}
    with state_lock:
        running_commands.append(cmd_info)
        
    try:
        proc = subprocess.Popen(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            cwd=WORKSPACE,
            bufsize=1
        )
        
        # Stream the output line-by-line in real-time
        for line in proc.stdout:
            if line.strip():
                stripped = line.strip()
                add_log(f" agy: {stripped}")
                with state_lock:
                    if tab_id not in tab_outputs:
                        tab_outputs[tab_id] = []
                    tab_outputs[tab_id].append(stripped)
                
        proc.wait()
        if proc.returncode == 0:
            add_log(f"agy finished successfully for tab: '{tab_title}'")
        else:
            add_log(f"agy error code {proc.returncode} for tab: '{tab_title}'")
    except Exception as e:
        add_log(f"Error running agy: {str(e)}")
    finally:
        with state_lock:
            if cmd_info in running_commands:
                running_commands.remove(cmd_info)

def poll_loop():
    global doc_title, last_polled, tabs_data, running
    
    while running:
        try:
            token = get_access_token()
            if not token:
                time.sleep(2)
                continue
                
            doc = fetch_document(token)
            
            with state_lock:
                doc_title = doc.get("title", "Unknown")
                last_polled = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
                
            current_tabs = []
            tabs = doc.get("tabs", [])
            
            def extract_all_tabs(tab_list):
                res_list = []
                for t in tab_list:
                    res_list.append(t)
                    child_tabs = t.get("childTabs", [])
                    if child_tabs:
                        res_list.extend(extract_all_tabs(child_tabs))
                return res_list
                
            flat_tabs = extract_all_tabs(tabs)
            for tab in flat_tabs:
                props = tab.get("tabProperties", {})
                tid = props.get("tabId")
                title = props.get("title", "")
                
                if "[agent,working]" in title:
                    status = "WORKING"
                elif "[agent]" in title:
                    status = "READY"
                elif "[human]" in title:
                    status = "HUMAN"
                else:
                    status = "Idle"
                    
                current_tabs.append({"id": tid, "title": title, "status": status})
                
            with state_lock:
                tabs_data = current_tabs
                
            # Process any READY tabs
            for tab in current_tabs:
                if tab["status"] == "READY":
                    tid = tab["id"]
                    old_title = tab["title"]
                    new_title = old_title.replace("[agent]", "[agent,working]")
                    
                    add_log(f"Renaming tab '{old_title}' -> '{new_title}'...")
                    try:
                        update_tab_title(tid, new_title, token)
                        add_log(f"Tab renamed to '{new_title}'")
                        
                        # Run the agy command in background thread
                        t = threading.Thread(target=run_agent_command, args=(new_title, tid))
                        t.daemon = True
                        t.start()
                    except Exception as e:
                        add_log(f"Rename Error: {str(e)}")
                        
        except Exception as e:
            add_log(f"Poll Error: {str(e)}")
            
        # Interruptible sleep
        for _ in range(POLL_INTERVAL * 2):
            if not running:
                break
            time.sleep(0.5)

def get_key():
    fd = sys.stdin.fileno()
    try:
        old_settings = termios.tcgetattr(fd)
    except termios.error:
        # Fallback if stdin is not a tty
        time.sleep(0.5)
        return None
        
    try:
        tty.setraw(fd)
        rlist, _, _ = select.select([sys.stdin], [], [], 0.1)
        if rlist:
            key = sys.stdin.read(1)
            if key == '\x1b':
                rlist, _, _ = select.select([sys.stdin], [], [], 0.05)
                if rlist:
                    next_char = sys.stdin.read(1)
                    key += next_char
                    if next_char == '[':
                        while True:
                            rlist, _, _ = select.select([sys.stdin], [], [], 0.05)
                            if rlist:
                                c = sys.stdin.read(1)
                                key += c
                                if c.isalpha():
                                    break
                            else:
                                break
                    elif next_char == 'O':
                        rlist, _, _ = select.select([sys.stdin], [], [], 0.05)
                        if rlist:
                            key += sys.stdin.read(1)
            return key
        return None
    finally:
        termios.tcsetattr(fd, termios.TCSADRAIN, old_settings)

def parse_mouse_click(key):
    match = re.match(r'^\x1b\[<(\d+);(\d+);(\d+)([Mm])$', key)
    if match:
        cb = int(match.group(1))
        cx = int(match.group(2))
        cy = int(match.group(3))
        event = match.group(4)
        return cb, cx, cy, event
    return None

def handle_input(key):
    global selected_selectable_idx, current_view, selected_tab_id, running
    
    selectables = []
    selectables.append({"type": "about", "label": "About", "id": "about"})
    selectables.append({"type": "logs", "label": "Global Activity Logs", "id": "logs"})
    for tab in tabs_data:
        selectables.append({"type": "tab", "label": f"Tab: {tab['title']}", "id": tab["id"]})
        
    if not selectables:
        return
        
    if key and key.startswith('\x1b[<'):
        click_info = parse_mouse_click(key)
        if click_info:
            cb, cx, cy, event = click_info
            if event == 'M' and cb == 0:  # Left click press
                if 1 <= cx <= 34:
                    # Click on left panel navigation menu (starts at row 12)
                    idx = cy - 12
                    if 0 <= idx < len(selectables):
                        selected_selectable_idx = idx
                        item = selectables[idx]
                        if item["type"] == "about":
                            current_view = "ABOUT"
                        elif item["type"] == "logs":
                            current_view = "LOGS"
                        elif item["type"] == "tab":
                            current_view = "TAB_OUTPUT"
                            selected_tab_id = item["id"]
                elif cx > 34 and current_view == "LOGS":
                    # Click on right panel tab status table (starts at row 3)
                    tab_idx = cy - 3
                    if 0 <= tab_idx < len(tabs_data):
                        clicked_tab = tabs_data[tab_idx]
                        for s_idx, s in enumerate(selectables):
                            if s["type"] == "tab" and s["id"] == clicked_tab["id"]:
                                selected_selectable_idx = s_idx
                                current_view = "TAB_OUTPUT"
                                selected_tab_id = clicked_tab["id"]
                                break
        return

    if key in ('\x1b[A', 'k', 'w'):  # Up Arrow
        selected_selectable_idx = (selected_selectable_idx - 1) % len(selectables)
    elif key in ('\x1b[B', 'j', 's'):  # Down Arrow
        selected_selectable_idx = (selected_selectable_idx + 1) % len(selectables)
    elif key in ('\r', '\n', ' '):  # Enter or Space
        selected_selectable_idx = min(selected_selectable_idx, len(selectables) - 1)
        item = selectables[selected_selectable_idx]
        if item["type"] == "about":
            current_view = "ABOUT"
        elif item["type"] == "logs":
            current_view = "LOGS"
        elif item["type"] == "tab":
            current_view = "TAB_OUTPUT"
            selected_tab_id = item["id"]
    elif key in ('q', 'Q', '\x03'):  # q or Ctrl+C
        running = False

def main():
    global running
    
    # Enable mouse reporting and SGR mode
    sys.stdout.write("\033[?1000h\033[?1006h")
    sys.stdout.flush()
    
    # Start polling thread
    poll_thread = threading.Thread(target=poll_loop)
    poll_thread.daemon = True
    poll_thread.start()
    
    try:
        while running:
            draw_ui()
            key = get_key()
            if key:
                handle_input(key)
    except KeyboardInterrupt:
        pass
    finally:
        running = False
        # Disable mouse reporting and SGR mode
        sys.stdout.write("\033[?1000l\033[?1006l")
        sys.stdout.write("\nExiting orchestrator...\n")
        sys.stdout.flush()

if __name__ == "__main__":
    main()
