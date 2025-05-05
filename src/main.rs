use std::cell::RefCell;
use std::io::{stdout};
use std::rc::Rc;
use sysinfo::{System, Pid};
use std::io;
use std::fs;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers,DisableMouseCapture,EnableMouseCapture},
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
    },
};
use ratatui::{
    layout::*, style::{Color, Modifier, Style}, symbols, text::{Line, Span,Text}, widgets::*, Frame,
    buffer::Buffer, backend::CrosstermBackend,   widgets::{Block, Borders, Paragraph},
    Terminal,
};
use std::path::Path;

use procfs::{process::Process, ProcResult, WithCurrentSystemInfo, Uptime, Current};
use nix::sys::{self, signal::{kill, Signal}};
use nix::unistd::Pid as NixPid;
use libc::{getpriority, PRIO_PROCESS, pid_t, c_int, syscall, SYS_tgkill};
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Eq, PartialEq)]
// struct representing each process and its information
struct TreeProc {
    pid: u32,
    ppid: u32,
    displayed: bool,
    children: Vec<Rc<RefCell<TreeProc>>>,
}

impl TreeProc {
    fn new(pid: u32, ppid: u32) -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(TreeProc {
            pid,
            ppid,
            displayed: false,
            children: Vec::new(),
        }))
    }

    // adds a child to the children 
    fn addchild(&mut self, child: Rc<RefCell<Self>>) {
        self.children.push(child);
    }

    // getter for the process id
    fn get_pid(&self) -> u32 {
        self.pid
    }

    // getter for the parent process id
    fn get_ppid(&self) -> u32 {
        self.ppid
    }

    // getter for the children vector for a process
    fn get_children(&self) -> Vec<Rc<RefCell<TreeProc>>> {
        self.children.clone()
    }

    // gets the number of children for a process
    fn get_numchildren(&self) -> usize {
        self.children.len()
    }
}

// creates a vector of Treeprocs and fills the in the information about each process
fn Tree_create() -> Vec<Rc<RefCell<TreeProc>>> {
    let mut system = System::new_all();
    system.refresh_all();

    // vector for all the processes in the system 
    let mut processes_running: Vec<Rc<RefCell<TreeProc>>> = Vec::new();

    // fills the pid and ppid for each process
    for (pid, process) in system.processes() {
        let parent_pid = process.parent().unwrap_or(0.into());
        let curr_proc = TreeProc::new(pid.as_u32(), parent_pid.as_u32());
        processes_running.push(curr_proc);
    }

    // creates a reference to each child of a process and adds them all to the children vector for each process
    for proc in &processes_running {
        for proc_child in &processes_running {
            if proc.borrow().get_pid() == proc_child.borrow().get_ppid() {
                proc.borrow_mut().addchild(Rc::clone(proc_child));
            }
        }
    }

    processes_running
}

// find the root process which is the parents of all parents
fn find_root(process_arr: &Vec<Rc<RefCell<TreeProc>>>) -> Option<usize> {
    for (index, proc) in process_arr.iter().enumerate() {
        // this process would have the parent id 0 so we look for it in the vector of processes
        if proc.borrow().get_ppid() == 0 {
            return Some(index);
        }
    }
    None
}

// creates the tree display of processes 
fn Tree_display(
    process_arr: Vec<Rc<RefCell<TreeProc>>>,  // Take ownership of the vector
    indent: usize,
    current: u32,
) -> Text<'static> {
    let mut tree_levels = Vec::new();

    // creates a line for each process
    for proc in process_arr {  
        let curr_proc = proc.borrow();
        let prefix = "   ".repeat(indent);

        // checks which process line is selected and highlights it
        let line = if current == curr_proc.get_pid() {
            Line::from(vec![
                Span::raw(prefix.clone()),
                Span::styled(
                    format!("{}", curr_proc.get_pid()),
                    Style::default()
                        .fg(Color::LightCyan)
                        .add_modifier(Modifier::BOLD),
                ),
            ])
        } 
        // if not selected then it does not get highlighted
        else {
            Line::from(vec![Span::raw(format!("{} -->  {}", prefix, curr_proc.get_pid()))])
        };

        tree_levels.push(line);

        
        let children = curr_proc.get_children();
        // recursively calls the function to display all the processes
        let children_text = Tree_display(children, indent + 1, current);
        tree_levels.extend(children_text.lines);
    }

    Text::from(tree_levels)
}

// does depth first search on each process and returns a vector of that order
fn stack_proc(procs: &mut Vec<Rc<RefCell<TreeProc>>>, parent: &Rc<RefCell<TreeProc>> ) {
    let proc = parent.borrow();
    for child in &proc.get_children() {
        procs.push(Rc::clone(&child));
        stack_proc(procs,child)
    }
}


/// Enum to define sorting modes for the process list
#[derive(PartialEq, Debug)]
enum SortMode {
    Cpu,
    Memory,
    Pid,
}

#[derive(PartialEq, Clone)]
enum Mode {
    Proc,
    Thread,
}

#[derive(Debug, Default, Clone)]
struct ThreadInfo {
    tid: u32,
    name: String,
    state: String,
    cpu: f64,
    priority: i64,

}

struct ThreadSample {
    last_cpu_time: u64,  // utime + stime
    last_seen: Instant,
}

/// Struct to manage application state
struct AppState {
    mode: Mode,
    proc_scroll_position: usize,
    thread_scroll_position: usize,
    proc_show_count: usize,
    thread_show_count: usize,
    proc_sort_mode: SortMode,
    thread_sort_mode: SortMode,
    frozen: bool,
    cached_pids: Option<Vec<Pid>>,
    cached_threads: Option<Vec<ThreadInfo>>,
    proc_selected_index: usize,  // Added for process selection
    thread_selected_index: usize,
    show_help: bool,        // Added for help panel toggle
    killed_pids: Vec<Pid>, // Track killed processes
    cpu_graph: Vec<(f64, f64)>, // track data points for graph 1
    memory_data: Vec<(f64, f64)>,    // Memory usage over time
    swap_data: Vec<(f64, f64)>,      // Swap usage over time
    disk_data: Vec<(f64, f64)>,      // Disk usage over time
    proc_tree: Vec<Rc<RefCell<TreeProc>>>, // contains the vector of processes with their children
    root_proc: Rc<RefCell<TreeProc>>, // gets the root process and its children
    curr_sel: u32, // gets the pid of the selected process

    thread_process_pid: Pid, // stores the process whose thread data is being displayed 
    thread_samples: HashMap<i32,ThreadSample>,
    latest_thread_count: usize,

}

impl AppState {
  fn new(
        proc_show_count: usize,
        thread_show_count: usize,
        proc_tree: Vec<Rc<RefCell<TreeProc>>>,
        root_proc: Rc<RefCell<TreeProc>>,
        curr_sel: u32,
    ) -> Self {
        Self {
            mode: Mode::Proc,
            proc_scroll_position: 0,
            thread_scroll_position: 0,
            proc_show_count,
            thread_show_count,
            proc_sort_mode: SortMode::Cpu,
            thread_sort_mode: SortMode::Cpu,
            frozen: false,
            cached_pids: None,
            cached_threads: None,
            proc_selected_index: 0,
            thread_selected_index: 0,
            show_help: false,
            killed_pids: Vec::new(),
            cpu_graph: Vec::new(),
            memory_data: Vec::new(),
            swap_data: Vec::new(),
            disk_data: Vec::new(),
            proc_tree,
            root_proc,
            curr_sel,
            thread_process_pid: Pid::from(1),
            thread_samples: HashMap::new(),
            latest_thread_count: 1,

        }
    }

    fn scroll_down(&mut self, total_processes: usize) {
        if self.proc_scroll_position + self.proc_show_count < total_processes {
            self.proc_scroll_position += 1;
        }
    }

    fn scroll_up(&mut self) {
        if self.proc_scroll_position > 0 {
            self.proc_scroll_position -= 1;
        }
    }

    fn page_down(&mut self, total_processes: usize) {

        if self.mode == Mode::Proc {
            // add the number of elements shown to the current scroll position
            let new_position = self.proc_scroll_position + self.proc_show_count;
            // select the minimum of the new position, and the largest possible position
            self.proc_scroll_position = std::cmp::min(new_position, total_processes.saturating_sub(self.proc_show_count));
        }
        else{
            // add the number of elements shown to the current scroll position
            let new_position = self.thread_scroll_position + self.thread_show_count;
            // select the minimum of the new position, and the largest possible position
            self.thread_scroll_position = std::cmp::min(new_position, self.latest_thread_count.saturating_sub(self.thread_show_count));
        }
    }

    fn page_up(&mut self) {
        if self.mode == Mode::Proc {
            self.proc_scroll_position = self.proc_scroll_position.saturating_sub(self.proc_show_count);
        }
        else {
            self.thread_scroll_position = self.thread_scroll_position.saturating_sub(self.thread_show_count);
        }
    }
    
    fn toggle_freeze(&mut self) {
        self.frozen = !self.frozen;
    }

    fn change_sort_mode(&mut self, sortmode: SortMode) {
        
        match self.mode
        {
            Mode::Proc => {
                self.proc_sort_mode = sortmode;
            // Reset cached processes when changing sort mode
                self.cached_pids = None;
            }
            Mode::Thread => {
                self.thread_sort_mode = sortmode;
                self.cached_threads = None;
            }
        }
    }
        
    
    // Added methods for selection navigation
    fn select_next(&mut self, total_processes: usize) {
        if self.mode == Mode::Proc {
            // if the selected index is within the bounds of currently visible elements and total number of processes, select next
            if self.proc_selected_index < self.proc_show_count - 1 && 
            self.proc_scroll_position + self.proc_selected_index + 1 < total_processes {
                self.proc_selected_index += 1;
            } // else if the next selected item is still within total number of elements, move the scroll position downwards
            else if self.proc_scroll_position + self.proc_show_count < total_processes {
                self.proc_scroll_position += 1;
            }
        }
        else{
            // if the selected index is within the bounds of currently visible elements and total number of processes, select next
            if self.thread_selected_index < self.thread_show_count - 1 && 
            self.thread_scroll_position + self.thread_selected_index + 1 < self.latest_thread_count {
                self.thread_selected_index += 1;
            } // else if the next selected item is still within total number of elements, move the scroll position downwards
            else if self.thread_scroll_position + self.thread_show_count < self.latest_thread_count {
                self.thread_scroll_position += 1;
            }
        }
    }
    
    fn select_previous(&mut self) {
        if self.mode == Mode::Proc {
            // if the to be selected item is in the list of showing elements
            if self.proc_selected_index > 0 {
                self.proc_selected_index -= 1;
            } // else if the to be selected element is within bounds
            else if self.proc_scroll_position > 0 {
                self.proc_scroll_position -= 1;
            }
        }
        else{
                // if the to be selected item is in the list of showing elements
                if self.thread_selected_index > 0 {
                    self.thread_selected_index -= 1;
                } // else if the to be selected element is within bounds
                else if self.thread_scroll_position > 0 {
                    self.thread_scroll_position -= 1;
                }
        }
    }
    
    fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
    }
}

// Function to send signals to the selected process
fn send_signal_to_selected_process(
    sys: &System,
    state: &mut AppState, // Mutable to update killed_pids
    signal: Signal
) -> Result<(), nix::Error> {
    if let Some(pids) = &state.cached_pids {
        if state.proc_scroll_position + state.proc_selected_index < pids.len() {
            let index = state.proc_scroll_position + state.proc_selected_index;
            let pid = pids[index];

            // Verify process still exists
            if sys.process(pid).is_none() {
                return Err(nix::Error::ESRCH);
            }

            let nix_pid = NixPid::from_raw(pid.as_u32() as i32);
            kill(nix_pid, signal)?;

            // Add process to killed list if SIGKILL or SIGTERM is sent
            if signal == Signal::SIGKILL || signal == Signal::SIGTERM {
                state.killed_pids.push(pid);
            }

            Ok(())
        } else {
            Err(nix::Error::ESRCH)
        }
    } else {
        Err(nix::Error::ESRCH)
    }
}

fn send_thread_signal( state: &mut AppState, signal: c_int) -> Result<(), nix::Error>  {

    if let Some(threads) = &state.cached_threads {
        if state.thread_scroll_position + state.thread_selected_index < state.latest_thread_count {
            let index = state.thread_scroll_position + state.thread_selected_index;
            let thread = &threads[index];

            let tgid = state.thread_process_pid;
            let result = unsafe { syscall(SYS_tgkill, tgid, thread.tid, signal) };

            if result == 0 {
                Ok(())
            } else {
                Err(nix::Error::ESRCH)
            }
        } else {
            Err(nix::Error::ESRCH)
            }
    } else {
        Err(nix::Error::ESRCH)
    }

    
   
}

// to get the selected process to display its threads
fn selected_pid(state: &AppState) -> Pid{
    if let Some(pids) = &state.cached_pids {
        let index = state.proc_scroll_position + state.proc_selected_index;
        if index < pids.len() {
            return pids[index]
        }
    }

    // Returns the currently selected PID from the cached list.
    // If the selection is invalid, returns PID 1 (init/systemd) as a failsafe.
    sysinfo::Pid::from(1)
}

fn help_panel<'a>() -> Paragraph<'a> {
    let left_text = vec![
        Line::from(Span::styled(
            "Process Management:",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )),
        Line::from("  K: Kill (SIGTERM)"),
        Line::from("  U: Force Kill (SIGKILL)"),
        Line::from("  S: Suspend"),
        Line::from("  R: Resume"),
        Line::from(""),
        Line::from(Span::styled(
            "Navigation:",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )),
        Line::from("  ↑/↓: Select Process"),
        Line::from("  PgUp/PgDn: Page Navigation"),
    ];

    let right_text = vec![
        Line::from(Span::styled(
            "Sorting:",
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        )),
        Line::from("  C: Sort by CPU Usage"),
        Line::from("  M: Sort by Memory Usage"),
        Line::from("  P: Sort by PID"),
        Line::from(""),
        Line::from(Span::styled(
            "Other:",
            Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD),
        )),
        Line::from("  F: Freeze Display"),
        Line::from("  Q: Quit Application"),
    ];

    let combined_text = left_text
        .into_iter()
        .chain(vec![Line::from("")]) // Add a blank line between columns
        .chain(right_text)
        .collect::<Vec<Line>>();

    Paragraph::new(combined_text)
        .block(
            Block::default()
                .title(Span::styled(
                    "Help",
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::ALL),
        )
        .wrap(Wrap { trim: true })
}


fn system_info(sys: &sysinfo::System) -> Table {
    let sys_titles = vec![
    "System Name:",
    "System Kernel Version:",
    "System OS Version:",
    "System Host Name:",
    "NB CPUs:",
    ];

    let sys_values = vec![
        System::name().unwrap_or("Unknown".to_string()),
        System::kernel_version().unwrap_or("Unknown".to_string()),
        System::os_version().unwrap_or("Unknown".to_string()),
        System::host_name().unwrap_or("Unknown".to_string()),
        format!("{}", sys.cpus().len()),
    ];

    let rows: Vec<Row> = sys_titles.iter().zip(sys_values.iter()).map(|(title, value)|{
        Row::new(vec![Cell::from(Span::raw(*title)),
                            Cell::from(Span::raw(value.clone())),])
    }).collect();


    Table::new(rows,
        &[ratatui::layout::Constraint::Percentage(50), 
        ratatui::layout::Constraint::Percentage(50)])
        .block(
            Block::default()
                .title(Span::styled(
                    "System Info", 
                    Style::default().add_modifier(Modifier::BOLD)
                ))
                .borders(Borders::ALL))
                   
}

fn usage_info(sys: &sysinfo::System) -> Table {
    let sys_titles = vec![
    "Total Memory:",
    "Used Memory:",
    "Total Swap:",
    "Used Swap:"
    ];

    let sys_values = vec![
        format!("{} MB", sys.total_memory() / (1024*1024)),
        format!("{} MB", sys.used_memory() / (1024*1024)),
        format!("{} MB", sys.total_swap() / (1024*1024)),
        format!("{} MB", sys.used_swap() / (1024*1024))
    ];

    let rows: Vec<Row> = sys_titles.iter().zip(sys_values.iter()).map(|(title, value)|{
        Row::new(vec![Cell::from(Span::raw(*title)),
                            Cell::from(Span::raw(value.clone())),])
    }).collect();


    Table::new(rows,
        &[ratatui::layout::Constraint::Percentage(65), 
        ratatui::layout::Constraint::Percentage(35)])
        .block(
            Block::default()
                .title(Span::styled(
                    "Usage Info", 
                    Style::default().add_modifier(Modifier::BOLD)
                ))
                .borders(Borders::ALL))
                   
}

fn cpu_info(sys: &sysinfo::System) -> Table{

    let mut rows: Vec<Row> = sys.cpus().chunks(2).enumerate().map(|(chunk_idx, chunk)|{
        
        let mut cells = Vec::new();

        for(i, cpu) in chunk.iter().enumerate(){
            let idx = chunk_idx * 2 + i;
            cells.push(Cell::from(Span::raw(format!("CPU {}:", idx))));
            cells.push(Cell::from(Span::raw(format!("{:.2}%", cpu.cpu_usage()))));
        }
        
        Row::new(cells)

    }).collect();

    let footer = Row::new(vec![
        Cell::from(Span::raw("Average CPU Usage")),
        Cell::from(Span::raw(format!("{:.2}%", sys.global_cpu_usage()))),
    ]);

    // go back to see dimensions
    Table::new(rows,
        &[ratatui::layout::Constraint::Percentage(20), 
        ratatui::layout::Constraint::Percentage(30),
        ratatui::layout::Constraint::Percentage(20),
        ratatui::layout::Constraint::Percentage(30)
        ])
        .block(
            Block::default()
                .title(Span::styled(
                    "Per CPU usage", 
                    Style::default().add_modifier(Modifier::BOLD)
                ))
                .borders(Borders::ALL)).footer(footer)
}

fn process_list<'a>(sys: &'a sysinfo::System, state: &'a mut AppState) -> Table<'a> {
    // Get processes to display
    let pids = if state.frozen && state.cached_pids.is_some() {
        // Use cached PIDs if frozen
        state.cached_pids.as_ref().unwrap().clone()
    } else {
        // Otherwise get fresh process list and sort
        let mut pids: Vec<Pid> = sys.processes().keys().copied().collect();
        
        // Sort based on selected sort mode
        match state.proc_sort_mode {
            SortMode::Cpu => {
                pids.sort_by(|a, b| {
                    let proc_a = sys.process(*a).unwrap();
                    let proc_b = sys.process(*b).unwrap();
                    proc_b.cpu_usage().partial_cmp(&proc_a.cpu_usage()).unwrap()
                });
            },
            SortMode::Memory => {
                pids.sort_by(|a, b| {
                    let proc_a = sys.process(*a).unwrap();
                    let proc_b = sys.process(*b).unwrap();
                    proc_b.memory().cmp(&proc_a.memory())
                });
            },
            SortMode::Pid => {
                pids.sort();
            }
        }
        
        // Cache the sorted list if not frozen
        if !state.frozen || state.cached_pids.is_none() {
            state.cached_pids = Some(pids.clone());
        }
    
        pids
    };
    
    // Get number of CPU cores for normalization
    let cpu_count = sys.cpus().len() as f32;
    
    // Calculate visible range based on scroll position
    let total = pids.len();
    let start = state.proc_scroll_position;
    let _end = std::cmp::min(start + state.proc_show_count, total);
    
    // Create rows only for visible processes with selection highlighting
    let rows: Vec<Row> = pids
        .iter()
        .skip(start)
        .take(state.proc_show_count)
        .enumerate()
        .filter_map(|(idx, pid)| {
            sys.process(*pid).map(|proc| {
                
                // top and htop display raw cpu so using that
                let normalized_cpu = proc.cpu_usage() / cpu_count;

                // Check if the process is marked as killed
                let status = if state.killed_pids.contains(pid) {
                    "Killed".to_string()
                } else {
                    format!("{}", proc.status())
                };

                
                // Highlight selected row
                let style = if idx == state.proc_selected_index && state.mode == Mode::Proc {
                    Style::default().bg(Color::Blue).fg(Color::White)
                } else {
                    Style::default()
                };

                let total_mem = sys.total_memory() as f64;
                Row::new(vec![
                    pid.to_string(),
                    proc.name().to_string_lossy().to_string(),
                    unsafe { getpriority(PRIO_PROCESS, pid.as_u32()) }.to_string(),
                    status,  // Display custom status here
                    format!("{:.2}%", proc.cpu_usage()),
                    format!("{:.2}%", (proc.memory() as f64 / total_mem) * 100.0 ),
                ]).style(style)
            })
        })
        .collect();


    // Create table with title indicating status and function keys
    let freeze_status = if state.frozen { " [FROZEN]" } else { "" };
    let proc_sort_mode = match state.proc_sort_mode {
        SortMode::Cpu => "CPU",
        SortMode::Memory => "MEM",
        SortMode::Pid => "PID",
    };
    
    let f_key_info = if state.show_help {
        ""  // If help panel is shown, don't crowd the title
    } else {
        " [H: Help]"  // Show minimal help when panel is hidden
    };
    
    Table::new(rows, [
        Constraint::Length(8),       // PID column width
        Constraint::Percentage(30),  // Process name column width
        Constraint::Length(5),       // Nice values column width 
        Constraint::Length(10),      // State column width
        Constraint::Length(12),      // CPU usage column width
        Constraint::Length(15),      // Memory usage column width
    ])
    .header(Row::new(vec![
        "PID".to_string(),
        "Process Name".to_string(),
        "NI".to_string(),
        "State".to_string(),       
        "CPU Usage".to_string(),
        "Memory Usage".to_string(),
    ]).style(Style::default().add_modifier(Modifier::BOLD)))
    .block(Block::default().title(format!(
        "Processes [{}] [Sort: {}{}]{}",
        total,
        proc_sort_mode,
        freeze_status,
        f_key_info
    )).borders(Borders::ALL))
    .column_spacing(1)
}

// Helper function to format bytes into human-readable units
fn bytes_to_human(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;

    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }

    format!("{:.1} {}", size, UNITS[unit_idx])
}

fn get_overall_process_data<'a>(sys: &'a sysinfo::System, app: &'a mut AppState)-> Table<'a> {
    
    let mut name = String::new();
    let mut memory = 0;

    if let Some(process) = sys.process(app.thread_process_pid)
    {
        name = process.name().to_string_lossy().into_owned();
        memory = process.memory();
    }
    
    let thread_count = fs::read_dir(format!("/proc/{}/task", app.thread_process_pid))
    .map(|dir| dir.count())
    .unwrap_or(0);

    app.latest_thread_count = thread_count;

    let rows = vec![
        Row::new(vec![
            Cell::from("Process name".to_string()),
            Cell::from(name),
        ]),
        Row::new(vec![
            Cell::from("Thread count".to_string()),
            Cell::from(thread_count.to_string()),
        ]),
        Row::new(vec![
            Cell::from("Memory".to_string()),
            Cell::from(bytes_to_human(memory)),
        ]),
    ];

    Table::new(rows,
        &[ratatui::layout::Constraint::Percentage(25), 
        ratatui::layout::Constraint::Percentage(75)])
        .block(Block::default()
        .title(Span::styled(
            "Process Data", 
            Style::default().add_modifier(Modifier::BOLD)
        ))
        .borders(Borders::ALL))

}



fn get_thread_info(state: & mut AppState) -> ProcResult<Vec<ThreadInfo>> {
    
    let pid = state.thread_process_pid.as_u32() as i32;
    let process = Process::new(pid)?;
    let tasks = process.tasks()?;
    let now = Instant::now();
    let clock_ticks = procfs::ticks_per_second() as f64;
            

    tasks
        .map(|task_result| {
            let task = task_result?; // Propagates task enumeration errors
            let stat = task.stat()?; // Propagates stat parsing errors
            let total_cpu= stat.utime + stat.stime;
            
            let mut cpu_percent= 0.0;

            if let Some(prev) = state.thread_samples.get(&task.tid) {
                let elapsed = now.duration_since(prev.last_seen).as_secs_f64();
                if elapsed > 0.0 {
                        let delta_cpu = (total_cpu - prev.last_cpu_time) as f64 / clock_ticks;
                        cpu_percent = 100.0 * (delta_cpu / elapsed);
                }
            }

            state.thread_samples.insert(task.tid, ThreadSample {
                last_cpu_time: total_cpu,
                last_seen: now,
            });


            Ok(ThreadInfo { // Wrap in Ok()
                tid: task.tid as u32,
                name: stat.comm,
                state: stat.state.to_string(),
                cpu: cpu_percent,
                priority: stat.priority,
            })
    })
    .collect()
}


fn thread_info_to_table<'a>(state: &'a mut AppState) -> Table<'a>{
    // Convert clock ticks to seconds (Linux default is 100 ticks/sec)
  

    let threads = if state.frozen && state.cached_threads.is_some() {
        // used cached threads if frozen
        state.cached_threads.as_ref().unwrap().clone()
    } else{ 
        let mut threads = get_thread_info(state).unwrap_or_default();

        // Sort based on selected sort mode
        match state.thread_sort_mode {
            SortMode::Cpu => {
                threads.sort_by(|a, b| b.cpu.partial_cmp(&a.cpu).unwrap_or(std::cmp::Ordering::Equal));
            },
            SortMode::Pid => {
                threads.sort_by(|a, b| a.tid.cmp(&b.tid));
            }
            SortMode::Memory => {
                threads.sort_by(|a, b| a.priority.cmp(&b.priority));
            }
        }

        // Cache the sorted list if not frozen
        if !state.frozen || state.cached_threads.is_none() {
            state.cached_threads = Some(threads.clone());
        }
    
        threads

        };

    let sort_mode = match state.thread_sort_mode {
        SortMode::Cpu => "CPU",
        SortMode::Pid => "TID",
        SortMode::Memory => "PRIO",
    };

    let header_style = Style::default()
        .fg(Color::LightCyan)
        .add_modifier(Modifier::BOLD);

    let start = state.thread_scroll_position;

    let rows: Vec<Row> = threads.iter()
    .skip(start)
    .take(state.thread_show_count)
    .enumerate()
    .map(|(idx,t)| 
        
        {

        // Highlight selected row
        let style = if idx == state.thread_selected_index && state.mode == Mode::Thread {
            Style::default().bg(Color::Blue).fg(Color::White)
        } else {
            Style::default()
        };

        Row::new(vec![
            Cell::from(t.tid.to_string()),
            Cell::from(t.name.clone()),
            Cell::from(t.state.clone()),
            Cell::from(format!("{:.2}%", t.cpu)),
            Cell::from(t.priority.to_string()),
        ]).style(style)
        }).collect();

        Table::new(rows, [
            Constraint::Length(6),   // TID
            Constraint::Length(16),  // Name
            Constraint::Length(6),   // State
            Constraint::Length(8),   // CPU
            Constraint::Length(5),   // Priority
        ])
        .header(
            Row::new(vec!["TID", "Name", "State", "CPU%", "Prio"])
                .style(header_style)
        )
        .block(Block::default().title(format!(
            "Thread Data [Sort: {}]",
            sort_mode,
        )).borders(Borders::ALL)
        )
        .column_spacing(1)
            
}


fn calculate_graph_x_ticks(area_width: u16)-> usize {
    let max_x_ticks = area_width;
    max_x_ticks as usize
}

fn calculate_x_bounds(data: &Vec<(f64, f64)>, x_ticks: usize)-> (f64, f64) {
    
    let x_start = data
        .get(data.len().saturating_sub(x_ticks))
        .map(|(x, _x)|*x)
        .unwrap_or(0.0);
    let x_end = if data.len() > x_ticks
        { data.last()
        .map(|(x, _x)|*x)
        .unwrap_or(x_ticks.saturating_sub(1) as f64)
        }
        else {x_ticks.saturating_sub(1) as f64};

    (x_start, x_end)
}

//  IGNORE this part but keep ot for now

// fn value_to_color(value: f64, max: f64) -> Color{
    
//     let ratio = value/max;
    
//     Color::Rgb(
//         (255.0 * ratio) as u8,       // Red component increases
//         (255.0 * (1.0 - ratio)) as u8, // Blue component decreases
//         100                            // Green fixed at 0
//     )
// }

// fn cpu_graph<'a>(sys: &'a sysinfo::System, app: &'a mut AppState, area: Rect) ->canvas::Canvas<'a, impl Fn(&mut canvas::Context) + 'a>{

//     // preparing the new point
//     let new_x = app.cpu_graph.last().map(|(x, _)| x + 1.0).unwrap_or(0.0);
//     let new_y = sys.global_cpu_usage() as f64;
//     app.cpu_graph.push((new_x, new_y));

//     let width = area.width; let height = area.height;
    
//     let x_ticks = calculate_graph_x_ticks(width);

//     let x_bounds = calculate_x_bounds(&app.cpu_graph, x_ticks);

//     let x_bounds = [x_bounds.0, x_bounds.1];


//     let data = &app.cpu_graph;


//     canvas::Canvas::default()
//     .block(Block::bordered().title("CPU usage").style(Style::default().bg(Color::Black)))
//     .x_bounds(x_bounds)
//     .y_bounds([0.0, 100.0])
//     .marker(symbols::Marker::Braille)
//     .paint(|ctx: &mut canvas::Context|
//     {
//         for (i, &(x, y)) in data.iter().enumerate(){
//             let color = value_to_color(y, 100.0);

//             for y_pos in 0..(y as usize) {
//                 ctx.print(
//                     i as f64 + 0.5,
//                     y_pos as f64 + 0.5,
//                     "â£¿".fg(color)
//                 );
//             }
//         }
//     })
// }

fn get_cpu_graph<'a>(sys: &'a sysinfo::System, app: &'a mut AppState, area: Rect) ->Chart<'a>{
    
    let new_x = app.cpu_graph.last().map(|(x, _)| x + 1.0).unwrap_or(0.0);
    let new_y = sys.global_cpu_usage() as f64;

    app.cpu_graph.push((new_x, new_y));

    let width = area.width;
    
    let x_ticks = calculate_graph_x_ticks(width);

    let x_bounds = calculate_x_bounds(&app.cpu_graph, x_ticks);

    let x_bounds = [x_bounds.0, x_bounds.1];

    if app.cpu_graph.len() > x_ticks{
        app.cpu_graph.remove(0);
    }


    let dataset = Dataset::default()
    .data(&app.cpu_graph)
    .graph_type(GraphType::Bar)
    .marker(symbols::Marker::Braille)
    .style(Style::default().fg(Color::Rgb(120, 200, 120)).bg(Color::Black));

    // Configure axes
    let x_axis = Axis::default()
        .bounds(x_bounds);

    let y_axis = Axis::default()
        .bounds([0.0, 100.0]);
        //.labels(vec!["0".into(), "50".into(), "100".into()]);

    // Build chart with both axes
    Chart::new(vec![dataset])
        .x_axis(x_axis)
        .y_axis(y_axis) 
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("CPU %"))
        .style(Style::default().bg(Color::Black))

}
struct MemoryGauges<'a> {
    total_mem: f64,
    used_mem: f64,
    available_mem: f64,
    cached_mem: f64,
    free_mem: f64,
    block: Block<'a>,
}
/// Returns (total, used, available, cached, free) in GiB, calculated exactly like btop
fn get_btop_memory_stats() -> (f64, f64, f64, f64, f64) {
    use std::fs::File;
    use std::io::{BufRead, BufReader};

    let mut memtotal = 0.0;
    let mut memfree = 0.0;
    let mut memavailable = 0.0;
    let mut cached = 0.0;
    let mut sreclaimable = 0.0;
    let mut shmem = 0.0;

    let file = File::open("/proc/meminfo").unwrap();
    let reader = BufReader::new(file);

    for line in reader.lines().flatten() {
        if line.starts_with("MemTotal:") {
            memtotal = line.split_whitespace().nth(1).unwrap_or("0").parse::<f64>().unwrap_or(0.0);
        } else if line.starts_with("MemFree:") {
            memfree = line.split_whitespace().nth(1).unwrap_or("0").parse::<f64>().unwrap_or(0.0);
        } else if line.starts_with("MemAvailable:") {
            memavailable = line.split_whitespace().nth(1).unwrap_or("0").parse::<f64>().unwrap_or(0.0);
        } else if line.starts_with("Cached:") && !line.contains("SwapCached") {
            cached = line.split_whitespace().nth(1).unwrap_or("0").parse::<f64>().unwrap_or(0.0);
        } else if line.starts_with("SReclaimable:") {
            sreclaimable = line.split_whitespace().nth(1).unwrap_or("0").parse::<f64>().unwrap_or(0.0);
        } else if line.starts_with("Shmem:") {
            shmem = line.split_whitespace().nth(1).unwrap_or("0").parse::<f64>().unwrap_or(0.0);
        }
    }

    // btop's formulas:
    let used = memtotal - memavailable;
    let cached_btop = cached + sreclaimable - shmem;

    // Convert from KiB to GiB (1 GiB = 1024^2 KiB)
    let kb_to_gib = 1024.0 * 1024.0;
    (
        memtotal / kb_to_gib,         // total
        used / kb_to_gib,             // used
        memavailable / kb_to_gib,     // available
        cached_btop / kb_to_gib,      // cached
        memfree / kb_to_gib           // free
    )
}


impl<'a> MemoryGauges<'a> {
    fn new(sys: &sysinfo::System) -> Self {
        let (total_mem, used_mem, available_mem, cached_mem, free_mem) = get_btop_memory_stats();
        Self {
            total_mem,
            used_mem,
            available_mem,
            cached_mem,
            free_mem,
            block: Block::default()
                .title("mem")
                .borders(Borders::ALL)
                .style(Style::default().bg(Color::Black)),
        }
    }
}
// btop-like colors
const MEM_USED_COLOR: Color = Color::Rgb(220, 70, 70);       // Red
const MEM_AVAILABLE_COLOR: Color = Color::Rgb(220, 220, 70);  // Yellow
const MEM_CACHED_COLOR: Color = Color::Rgb(70, 70, 220);      // Blue
const MEM_FREE_COLOR: Color = Color::Rgb(70, 220, 70);        // Green
const DISK_IO_COLOR: Color = Color::Rgb(220, 200, 80);        // Yellow
const DISK_USED_COLOR: Color = Color::Rgb(220, 70, 70);       // Red
const DISK_FREE_COLOR: Color = Color::Rgb(70, 200, 70);       // Green

impl<'a> Widget for MemoryGauges<'a> {
    fn render(self, rect: Rect, buf: &mut Buffer) {
        let used_percent = (self.used_mem / self.total_mem) * 100.0;
        let available_percent = (self.available_mem / self.total_mem) * 100.0;
        let cached_percent = (self.cached_mem / self.total_mem) * 100.0;
        let free_percent = (self.free_mem / self.total_mem) * 100.0;

        let inner = self.block.inner(rect);
        self.block.render(rect, buf);

        // Title with total memory at top
        Paragraph::new(format!("Total: {:.1} GiB", self.total_mem))
            .style(Style::default().fg(Color::White))
            .render(Rect::new(inner.x, inner.y, inner.width, 1), buf);

        // Each metric takes 2 lines
        let memory_y = [
            inner.y + 1,  // Used
            inner.y + 4,  // Available
            inner.y + 7,  // Cached
            inner.y + 10,  // Free
        ];
        
        render_memory_line(
            Rect::new(inner.x, memory_y[0], inner.width, 2),
            "Used:", 
            used_percent, 
            self.used_mem,
            MEM_USED_COLOR,
            buf,
        );
        render_memory_line(
            Rect::new(inner.x, memory_y[1], inner.width, 2),
            "Available:", 
            available_percent, 
            self.available_mem,
            MEM_AVAILABLE_COLOR,
            buf,
        );
        render_memory_line(
            Rect::new(inner.x, memory_y[2], inner.width, 2),
            "Cached:", 
            cached_percent, 
            self.cached_mem,
            MEM_CACHED_COLOR,
            buf,
        );
        render_memory_line(
            Rect::new(inner.x, memory_y[3], inner.width, 2),
            "Free:", 
            free_percent, 
            self.free_mem,
            MEM_FREE_COLOR,
            buf,
        );
    }
}


fn render_memory_line(area: Rect, label: &str, percent: f64, value: f64, color: Color, buf: &mut Buffer) {
    // First line: label left, value right
    Paragraph::new(format!("{:<9}{:>8.2} GiB", label, value))
        .style(Style::default().fg(color))
        .render(Rect::new(area.x, area.y, area.width, 1), buf);

    // Second line: graph left, percent right
    let bar_width = area.width.saturating_sub(6);
    render_braille_gauge(
        Rect::new(area.x, area.y + 1, bar_width, 1),
        percent / 100.0,
        color,
        buf,
    );
    Paragraph::new(format!("{:>3.0}%", percent))
        .style(Style::default().fg(color))
        .alignment(Alignment::Right)
        .render(Rect::new(area.x + bar_width, area.y + 1, 6, 1), buf);
}


fn memory_gauges<'a>(sys: &sysinfo::System) -> MemoryGauges {
    MemoryGauges::new(sys)
}
struct DiskGauges<'a> {
    sys: &'a sysinfo::System,
    block: Block<'a>,
}


impl<'a> DiskGauges<'a> {
    fn new(sys: &'a sysinfo::System) -> Self {
        Self {
            sys,
            block: Block::default()
                .title("disks")
                // .border_style(Color::Rgb((20), (30), (40)))
                .borders(Borders::ALL)
                .style(Style::default().bg(Color::Black)),
        }
    }
}
fn render_braille_gauge(area: Rect, ratio: f64, color: Color, buf: &mut Buffer) {
    // Ensure we're within buffer bounds
    let area = area.intersection(*buf.area());
    if area.area() == 0 {
        return;
    }
    
    let width = area.width as usize;
    
    // These characters create a dotted pattern with increasing density
    let braille_chars = [
        '⠀', '⠁', '⠉', '⠋', '⠛', '⠟', '⠿', '⣿'
    ];
    
    // Calculate how many dots will be filled
    let filled_width = (ratio * width as f64) as usize;
    
    // Calculate partial fill for the last character
    let partial_char_idx = ((ratio * width as f64 * 8.0) as usize) % 8;
    
    // Render each dot position
    for x in 0..width {
        let pos = area.x + x as u16;
        if x < filled_width {
            // Use the modern buffer indexing syntax
            buf[(pos, area.y)].set_char('⣿').set_fg(color);
        } else if x == filled_width && partial_char_idx > 0 {
            buf[(pos, area.y)].set_char(braille_chars[partial_char_idx]).set_fg(color);
        } else {
            buf[(pos, area.y)].set_char('⠀').set_fg(color.darker(50));
        }
    }
}

fn render_disk_line(
    area: Rect,
    label: &str,
    percent: f64,
    value: f64,
    color: Color,
    buf: &mut Buffer,
) {
    // First line: label left, value right
    Paragraph::new(format!("{:<8}{:>8.2} GiB", label, value))
        .style(Style::default().fg(color))
        .render(Rect::new(area.x, area.y, area.width, 1), buf);

    // Second line: bar and percent
    let bar_width = area.width.saturating_sub(6);
    render_braille_gauge(
        Rect::new(area.x, area.y + 1, bar_width, 1),
        percent / 100.0,
        color,
        buf,
    );
    Paragraph::new(format!("{:>3.0}%", percent))
        .style(Style::default().fg(color))
        .alignment(Alignment::Right)
        .render(Rect::new(area.x + bar_width, area.y + 1, 6, 1), buf);
}

impl<'a> Widget for DiskGauges<'a> {
    fn render(self, rect: Rect, buf: &mut Buffer) {
        let inner = self.block.inner(rect);
        self.block.render(rect, buf);
        
        // Spacing constants for each disk entry
        const DISK_ENTRY_HEIGHT: u16 = 4; // Title + IO% + Used + Free
        
        // Calculate how many disks we show
        let disks = sysinfo::Disks::new_with_refreshed_list();
        let mut y_offset = inner.y;
        
        // First render root disk (if exists)
        if let Some(disk) = disks.iter().find(|d| d.mount_point().to_string_lossy() == "/") {
            let total = disk.total_space() as f64 / 1_073_741_824.0; // GiB
            let free = disk.available_space() as f64 / 1_073_741_824.0;
            let used = total - free;
            let used_percent = (used / total) * 100.0;
            let free_percent = 100.0 - used_percent;
            
            // Title line with total size
            Paragraph::new(format!("root {:.0} GiB", total))
                .style(Style::default().fg(Color::White))
                .render(Rect::new(inner.x, y_offset + 1, inner.width, 1), buf);;
            
            // Used line
            Paragraph::new(format!("Used: {:.0}%", used_percent))
            .style(Style::default().fg(MEM_USED_COLOR))
            .render(Rect::new(inner.x, y_offset + 2, 10, 1), buf);
            
            render_braille_gauge(
                Rect::new(inner.x + 10, y_offset + 2, inner.width.saturating_sub(20), 1),
                used_percent / 100.0,
                MEM_USED_COLOR,
                buf
            );
            
            Paragraph::new(format!("{:.1} GiB", used))
                .style(Style::default().fg(Color::Gray))
                .alignment(Alignment::Right)
                .render(Rect::new(inner.x + inner.width.saturating_sub(10), y_offset + 2, 10, 1), buf);
            
            // Free line
            Paragraph::new(format!("Free: {:.0}%", free_percent))
                .style(Style::default().fg(Color::Green))
                .render(Rect::new(inner.x, y_offset + 3, 10, 1), buf);
            
            render_braille_gauge(
                Rect::new(inner.x + 10, y_offset + 3, inner.width.saturating_sub(20), 1),
                free_percent / 100.0,
                Color::Green,
                buf
            );
            
            Paragraph::new(format!("{:.1} GiB", free))
                .style(Style::default().fg(Color::Gray))
                .alignment(Alignment::Right)
                .render(Rect::new(inner.x + inner.width.saturating_sub(10), y_offset + 3, 10, 1), buf);
            
            y_offset += DISK_ENTRY_HEIGHT + 1; // Add spacing between entries
        }
        
        // Then render swap if it exists
        if self.sys.total_swap() > 0 {
            let total_swap = self.sys.total_swap() as f64 / 1_073_741_824.0; // GiB
            let used_swap = self.sys.used_swap() as f64 / 1_073_741_824.0;
            let used_percent = if total_swap > 0.0 { (used_swap / total_swap) * 100.0 } else { 0.0 };
            let free_percent = 100.0 - used_percent;
            
            // Title line
            Paragraph::new(format!("swap {:.2} GiB", total_swap))
                .style(Style::default().fg(Color::White))
                .render(Rect::new(inner.x, y_offset, inner.width, 1), buf);
            
            // Used line
            Paragraph::new(format!("Used: {:.0}%", used_percent))
                .style(Style::default().fg(Color::Red))
                .render(Rect::new(inner.x, y_offset + 1, 10, 1), buf);
            
            render_braille_gauge(
                Rect::new(inner.x + 10, y_offset + 1, inner.width.saturating_sub(20), 1),
                used_percent / 100.0,
                Color::Red,
                buf
            );
            
            Paragraph::new(format!("{:.1} GiB", used_swap))
                .style(Style::default().fg(Color::Gray))
                .alignment(Alignment::Right)
                .render(Rect::new(inner.x + inner.width.saturating_sub(10), y_offset + 1, 10, 1), buf);
            
            // Free line
            Paragraph::new(format!("Free: {:.0}%", free_percent))
                .style(Style::default().fg(Color::Green))
                .render(Rect::new(inner.x, y_offset + 2, 10, 1), buf);
            
            render_braille_gauge(
                Rect::new(inner.x + 10, y_offset + 2, inner.width.saturating_sub(20), 1),
                free_percent / 100.0,
                Color::Green,
                buf
            );
            
            Paragraph::new(format!("{:.1} GiB", total_swap - used_swap))
                .style(Style::default().fg(Color::Gray))
                .alignment(Alignment::Right)
                .render(Rect::new(inner.x + inner.width.saturating_sub(10), y_offset + 2, 10, 1), buf);
        }
    }
}

fn disk_gauges(sys: &sysinfo::System) -> DiskGauges {
    DiskGauges::new(sys)
}

fn draw_ui(sys: &sysinfo::System, state: &mut AppState, frame: &mut Frame, tree: bool) {
    // Get dynamic terminal size
    let area = frame.area();

    // Background
    let background = Block::default()
        .style(Style::default().bg(Color::Black));
    frame.render_widget(background, area);

    // If tree mode is enabled, draw tree and return early
    if tree {
        let tree_proc = vec![Rc::clone(&state.root_proc)];
        let tree_text = Tree_display(tree_proc, 0, state.curr_sel);
        let tree_widget = Paragraph::new(tree_text)
            .block(Block::default().title("Process Tree").borders(Borders::ALL))
            .style(Style::default().bg(Color::Black));
        frame.render_widget(tree_widget, area);
        return;
    }

    if state.show_help {
        // For help layout: calculate available space after system stats and help panel
        let available_height = area.height.saturating_sub(12 + 20 + 3);
        state.proc_show_count = available_height as usize;

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(12),  // System stats
                Constraint::Length(20),  // Help panel 
                Constraint::Min(5),      // Process list
            ])
            .split(area);

        let upper_list_layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(vec![
                Constraint::Percentage(36),
                Constraint::Percentage(25),
                Constraint::Percentage(39),
            ])
            .split(layout[0]);

        frame.render_widget(system_info(&sys), upper_list_layout[0]);
        frame.render_widget(usage_info(&sys), upper_list_layout[1]);
        frame.render_widget(cpu_info(&sys), upper_list_layout[2]);
        frame.render_widget(help_panel(), layout[1]);
        frame.render_widget(process_list(&sys, state), layout[2]);
    } else {
        
        let [top, middle, bottom] = Layout::vertical([
            Constraint::Fill(1),
            Constraint::Fill(2),
            Constraint::Fill(2)
        ]).areas(area);

        let [systeminfo, useageinfo] = Layout::horizontal([
            Constraint::Fill(1),
            Constraint::Fill(1)
        ]).areas(top);

        let [cpu, mem_disk] = Layout::horizontal([
            Constraint::Fill(2),
            Constraint::Fill(1)
        ]).areas(middle);

        let [cpus, cpu_graph] = Layout::horizontal([
            Constraint::Fill(2),
            Constraint::Fill(3)
        ]).areas(cpu);

        let [mem, disk] = Layout::vertical([
            Constraint::Fill(1),
            Constraint::Fill(1)
        ]).areas(mem_disk);

        let [process, thread] = Layout::horizontal([
            Constraint::Fill(1),
            Constraint::Fill(1)
        ]).areas(bottom);

        let [thread_general, per_thread] = Layout::vertical([
            Constraint::Fill(1),
            Constraint::Fill(4)
        ]).areas(thread);


        let process_section_height = (process.height as f32).floor() as u16;
        state.proc_show_count = ((process_section_height.saturating_sub(3)) as f64) as usize;

        let thread_section_height = (per_thread.height as f32).floor() as u16;
        state.thread_show_count = ((thread_section_height.saturating_sub(3)) as f64) as usize;
        frame.render_widget(system_info(&sys), top);
        // frame.render_widget(usage_info(&sys), useageinfo);
        frame.render_widget(cpu_info(&sys), cpus);
        frame.render_widget(get_cpu_graph(&sys, state, cpu_graph), cpu_graph);
        frame.render_widget(memory_gauges(&sys), mem);
        frame.render_widget(disk_gauges(&sys), disk);
        frame.render_widget(process_list(&sys, state), process);
        frame.render_widget(get_overall_process_data(&sys, state), thread_general);
        frame.render_widget(thread_info_to_table(state), per_thread);
        
    }
}

// Extension for colors to create darker/lighter variants
trait ColorExt {
    fn darker(&self, amount: u8) -> Self;
    fn lighter(&self, amount: u8) -> Self;
}

impl ColorExt for Color {
    fn darker(&self, amount: u8) -> Self {
        match self {
            Color::Rgb(r, g, b) => {
                Color::Rgb(
                    r.saturating_sub(amount),
                    g.saturating_sub(amount),
                    b.saturating_sub(amount),
                )
            }
            _ => *self,
        }
    }
    
    fn lighter(&self, amount: u8) -> Self {
        match self {
            Color::Rgb(r, g, b) => {
                Color::Rgb(
                    r.saturating_add(amount),
                    g.saturating_add(amount),
                    b.saturating_add(amount),
                )
            }
            _ => *self,
        }
    }
}


fn main() -> io::Result<()> {
    // Initialize terminal
    let mut terminal = ratatui::init();

    let mut sys = System::new_all();
    
    
   let processes_tree: Vec<Rc<RefCell<TreeProc>>> = Tree_create();
   let root_index = find_root(&processes_tree).unwrap();
   let root_proc = Rc::clone(&processes_tree[root_index]); 


let mut state = AppState::new(15, 15, processes_tree, Rc::clone(&root_proc), root_proc.borrow().get_pid());

    
    let mut tree:bool = false;
    let mut i = 0; // index into the  tree stack
    let mut stack: Vec<Rc<RefCell<TreeProc>>> = vec![Rc::clone(&state.root_proc)];
    
    
    // Give system time to collect baseline metrics
    sys.refresh_all();
    std::thread::sleep(std::time::Duration::from_millis(500));


    loop {
        // Only refresh if not frozen
        if !state.frozen {
            sys.refresh_all();
        }
        
        let total_processes = sys.processes().len();

        terminal.draw(|frame| draw_ui(&sys, & mut state, frame,tree))?;

        // Handle keyboard input for scrolling and process management
        if crossterm::event::poll(std::time::Duration::from_millis(750))? {
            if let Event::Key(key) = crossterm::event::read()? {
                match key.code {
                    // Navigation keys
                    KeyCode::Char('q') => break,
                    KeyCode::Char('f') => state.toggle_freeze(),
                    KeyCode::Char('c') => state.change_sort_mode(SortMode::Cpu),
                    KeyCode::Char('m') => state.change_sort_mode(SortMode::Memory),
                    KeyCode::Char('p') => state.change_sort_mode(SortMode::Pid),
                    
                    // Selection navigation
                    KeyCode::Down => {
                        if tree{
                            if i + 1 >= stack.len() {
                                i = 0;
                            }
                        else{
                            i = i + 1;
                            }
                        state.curr_sel = stack[i].borrow().get_pid();
                        }
                        else{
                            state.select_next(total_processes)
                        }
                    },
                    KeyCode::Up => {
                        if tree{
                            if i == 0 {
                                i = stack.len() - 1 ; 
                                }
                            else if i != 0{
                                i = i - 1;
                                }
                            state.curr_sel = stack[i].borrow().get_pid();
                        }
                        else{
                            state.select_previous()
                        }
                    },

                    KeyCode::Right => 
                    {
                        state.mode = Mode::Thread;
                    },
                    KeyCode::Left => state.mode = Mode::Proc,

                    KeyCode::PageDown => state.page_down(total_processes),
                    KeyCode::PageUp => state.page_up(),
                    
                    // Process management
                    KeyCode::Char('h') => state.toggle_help(),
                    KeyCode::Char('k') => {
                        if state.mode == Mode::Proc {
                            if let Err(e) = send_signal_to_selected_process(&sys, &mut state, Signal::SIGTERM) {
                                eprintln!("Error sending SIGTERM: {}", e);
                            }
                        }
                    },
                    KeyCode::Char('u') => {
                        if state.mode == Mode::Proc {
                            if let Err(e) = send_signal_to_selected_process(&sys, &mut state, Signal::SIGKILL) {
                                eprintln!("Error sending SIGKILL: {}", e);
                            }
                        }
                        else{
                            if let Err(e) = send_thread_signal(& mut state, libc::SIGKILL) {
                                eprintln!("Error sending SIGKILL: {}", e);
                            }
                        }
                    },                    
                    KeyCode::Char('s') => {
                        if state.mode == Mode::Proc {
                            if let Err(e) = send_signal_to_selected_process(&sys, &mut state, Signal::SIGSTOP) {
                                eprintln!("Error sending SIGSTOP: {}", e);
                            }
                        }  
                        else{
                            if let Err(e) = send_thread_signal(& mut state, libc::SIGSTOP) {
                                eprintln!("Error sending SIGSTOP: {}", e);
                            }
                        }
                    },
                    KeyCode::Char('r') => {
                        if state.mode == Mode::Proc {
                            if let Err(e) = send_signal_to_selected_process(&sys, &mut state, Signal::SIGCONT) {
                                eprintln!("Error sending SIGCONT: {}", e);
                            }
                        }
                        else{
                            if let Err(e) = send_thread_signal(& mut state, libc::SIGCONT) {
                                eprintln!("Error sending SIGCONT: {}", e);
                            }
                        }
                    },
                    
                    KeyCode::Char('t') if key.modifiers.is_empty() => {
                        tree = !tree;
                        stack_proc(&mut stack, &state.root_proc.clone());
                    },
                    KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        let pt_pid = selected_pid(&state);
                        if Path::new(&format!("/proc/{}", pt_pid)).exists(){
                            state.thread_process_pid = pt_pid;
                            state.thread_scroll_position = 0;
                            state.thread_selected_index = 0;
                            state.cached_threads = None;
                        }
                    },
                    _ => {}
                }
                
            }
        }
    }

    ratatui::restore();
    Ok(())
}

           