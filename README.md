# Google Docs Agent Orchestrator TUI (Rust Edition)

A high-performance, concurrent terminal user interface (TUI) built in Rust to orchestrate, monitor, and stream background Gemini-powered Google Doc agents running via the `agy` CLI tool.

## Key Features
* **Multithreaded Concurrent Execution**: Avoids UI blocking by isolating user input, Google Docs API polling, process spawning, and stdout/stderr stream capturing into distinct threads.
* **Pure Standard Library**: Built with exactly **zero external dependencies** (`Cargo.toml` remains dependency-free) for lightning-fast compilation, portability, and 100% offline-ready operations on any standard host platform.
* **Rich Layout**: Native split-column view (32-column left navigation, 52-column right information/logs card).
* **Interactive SGR Mouse Support**: Fully interactive keyboard navigation fallback with real-time mouse-click detection. Clicking navigation items or table rows immediately updates view contexts.
* **Hierarchical Tab Extraction**: Auto-traverses and retrieves deep nested Google Doc sub-tabs recursively using a highly optimized JSON property extraction scanner.

---

## Architecture Diagram

The diagram below details the simplified architecture, state synchronization, and execution boundaries:

```mermaid
graph TD
    subgraph Host ["Host System"]
        subgraph TUI ["Rust TUI Process"]
            UI["Render & Input Engine"]
            Poll["Poll & Execution Coordinator"]
            State[("Shared AppState")]
        end
        
        agy["agy Agent CLI"]
        tools["Local CLI Tools<br>(ffmpeg, gcc, git, etc.)"]
    end
    
    subgraph GCP ["Google Cloud Platform"]
        DocsAPI["Google Docs API"]
        GCPServices["GCP Services<br>(Vertex AI, Cloud Storage, BigQuery, etc.)"]
    end
    
    %% Inputs
    User["Keyboard & Mouse"] -->|Input Events| UI
    UI <-->|Read/Write| State
    
    %% Polling & API Communication
    Poll -->|Fetch Tabs & Update Title| DocsAPI
    Poll <-->|Read/Write Status| State
    
    %% Spawning agy
    Poll -->|Spawn Agent Run| agy
    agy -->|Real-time stdout/stderr| State
    
    %% agy interaction with host tools and other GCP services
    agy <-->|Execute Commands| tools
    agy <-->|API Calls| GCPServices
```

---

## Execution Thread Breakdown

1. **Main Render Thread**: Triggers drawing loops, parses SGR terminal coordinate mouse packets, coordinates navigation selections, and writes explicit Carriage Returns (`\r\n`) to support raw terminal modes without layout distortion.
2. **Stdin Input Thread**: Non-blocking input worker that reads bytes from standard input and channels them to the Main Render Thread via an asynchronous `std::sync::mpsc::channel`.
3. **Background Poll Loop Thread**: Wakes up every 5 seconds to query the local `gcloud` CLI for fresh credentials and fetch the hierarchical document structure via `curl`. If a tab is found in `READY` status, renames the tab to `WORKING` and triggers a background agent thread.
4. **Background agy Task Thread**: Manages the child lifecycle of the `agy` process, spawning dedicated concurrent reader threads for `stdout` and `stderr` streams to aggregate terminal outputs in real-time.

---

## Getting Started

1. **Launch the TUI**:
   ```bash
   ./start_tui.sh
   ```
2. **Controls**:
   * **Mouse**: Click on options on the left-hand menu, or on any tab listed in the `Document Tabs Status` table to view real-time logs.
   * **Keyboard**: Navigate using arrow keys (or `w`/`s`, `j`/`k`), choose using `[Enter]` or `[Space]`.
   * **Exit**: Press `[q]` or `[Ctrl+C]`.
