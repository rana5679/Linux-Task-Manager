use sysinfo::{System, Pid};
use std::io;
use crossterm::event::{Event, KeyCode};
use ratatui::{
    layout::*,
    widgets::*,
    style::{Style, Modifier, Color},
    text::{Span, Line},
    symbols,
};
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid as NixPid;
use libc::{getpriority, PRIO_PROCESS};

/// Enum to define sorting modes for the process list
enum SortMode {
    Cpu,
    Memory,
    Pid,
}

/// Struct to manage application state
struct AppState {
    scroll_position: usize,
    show_count: usize,
    sort_mode: SortMode,
    frozen: bool,
    cached_pids: Option<Vec<Pid>>,
    selected_index: usize,  // Added for process selection
    show_help: bool,        // Added for help panel toggle
    killed_pids: Vec<Pid>, // Track killed processes
    g1_data: Vec<(f64, f64)>, // track data points for graph 1
}

impl AppState {
    fn new(show_count: usize) -> Self {
        Self {
            scroll_position: 0,
            show_count,
            sort_mode: SortMode::Cpu,
            frozen: false,
            cached_pids: None,
            selected_index: 0,
            show_help: false,
            killed_pids: Vec::new(), 
            g1_data: Vec::new(),
        }
    }

    fn scroll_down(&mut self, total_processes: usize) {
        if self.scroll_position + self.show_count < total_processes {
            self.scroll_position += 1;
        }
    }

    fn scroll_up(&mut self) {
        if self.scroll_position > 0 {
            self.scroll_position -= 1;
        }
    }

    fn page_down(&mut self, total_processes: usize) {
        // add the number of elements shown to the current scroll position
        let new_position = self.scroll_position + self.show_count;
        // select the minimum of the new position, and the largest possible position
        self.scroll_position = std::cmp::min(new_position, total_processes.saturating_sub(self.show_count));
    }

    fn page_up(&mut self) {
        self.scroll_position = self.scroll_position.saturating_sub(self.show_count);
    }
    
    fn toggle_freeze(&mut self) {
        self.frozen = !self.frozen;
    }

    fn change_sort_mode(&mut self, mode: SortMode) {
        self.sort_mode = mode;
        // Reset cached processes when changing sort mode
        self.cached_pids = None;
    }
    
    // Added methods for selection navigation
    fn select_next(&mut self, total_processes: usize) {
        // if the selected index is within the bounds of currently visible elements and total number of processes, select next
        if self.selected_index < self.show_count - 1 && 
           self.scroll_position + self.selected_index + 1 < total_processes {
            self.selected_index += 1;
        } // else if the next selected item is still within total number of elements, move the scroll position downwards
         else if self.scroll_position + self.show_count < total_processes {
            self.scroll_position += 1;
        }
    }
    
    fn select_previous(&mut self) {
        // if the to be selected item is in the list of showing elements
        if self.selected_index > 0 {
            self.selected_index -= 1;
        } // else if the to be selected element is within bounds
         else if self.scroll_position > 0 {
            self.scroll_position -= 1;
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
        if state.scroll_position + state.selected_index < pids.len() {
            let index = state.scroll_position + state.selected_index;
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
        ratatui::layout::Constraint::Percentage(30)])
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
        match state.sort_mode {
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
    let start = state.scroll_position;
    let _end = std::cmp::min(start + state.show_count, total);
    
    // Create rows only for visible processes with selection highlighting
    let rows: Vec<Row> = pids
        .iter()
        .skip(start)
        .take(state.show_count)
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
                let style = if idx == state.selected_index {
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
    let sort_mode = match state.sort_mode {
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
        sort_mode,
        freeze_status,
        f_key_info
    )).borders(Borders::ALL))
    .column_spacing(1)
}

fn cpu_graph<'a>(sys: &'a sysinfo::System, app: &'a mut AppState) ->Chart<'a>{
    let new_x = app.g1_data.last().map(|(x, _)| x + 1.0).unwrap_or(0.0);
    let new_y = sys.global_cpu_usage() as f64;

    app.g1_data.push((new_x, new_y));


    let dataset = Dataset::default()
    .data(&app.g1_data)
    .graph_type(GraphType::Bar)
    .marker(symbols::Marker::Braille);


    let x_bounds = if app.g1_data.len() > 10 {
        [app.g1_data[0].0, app.g1_data.last().unwrap().0]
    } else {
        [0.0, 10.0] // Default if < 2 points
    };


    // Configure axes
    let x_axis = Axis::default()
        .title("Time")
        .bounds(x_bounds);

    let y_axis = Axis::default()
        .title("CPU %")
        .bounds([0.0, 100.0]);
        //.labels(vec!["0".into(), "50".into(), "100".into()]);

    // Build chart with both axes
    Chart::new(vec![dataset])
        .x_axis(x_axis)
        .y_axis(y_axis) 
        .block(
            Block::default()
                .title(Span::styled(
                    "Per CPU usage", 
                    Style::default().add_modifier(Modifier::BOLD)
                ))
                .borders(Borders::ALL)
                .style(Style::default().bg(Color::Black)))

}

fn main() -> io::Result<()> {
    // Initialize terminal
    let mut terminal = ratatui::init();

    let mut sys = System::new_all();
    
    let mut state = AppState::new(15);
    
    // Give system time to collect baseline metrics
    sys.refresh_all();
    std::thread::sleep(std::time::Duration::from_millis(500));


    loop {
        // Only refresh if not frozen
        if !state.frozen {
            sys.refresh_all();
        }
        
        let total_processes = sys.processes().len();

        terminal.draw(|frame| {
            // Different layout based on whether help is showing
            if state.show_help {
                // Layout with help panel
                let layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(12),  // System stats
                    // Constraint::Length(20),  // cpu info
                    Constraint::Length(20),  // Help panel 
                    Constraint::Min(5),      // Process list
                ])
                .split(frame.area());

                let upper_list_layout = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(vec![
                    Constraint::Percentage(36),
                    Constraint::Percentage(25),
                    Constraint::Percentage(39),
                ])
                .split(layout[0]);

                let background = Block::default()
                .style(Style::default().bg(Color::Black));
                frame.render_widget(background, frame.area());

                frame.render_widget(system_info(&sys), upper_list_layout[0]);
                frame.render_widget(usage_info(&sys), upper_list_layout[1]);
                frame.render_widget(cpu_info(&sys), upper_list_layout[2]);
                frame.render_widget(help_panel(), layout[1]);
                frame.render_widget(process_list(&sys, &mut state), layout[2]);
            } else {
                // Standard layout without help
                let layout = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Percentage(25), // System stats
                        Constraint::Percentage(50),  // cpu info
                        Constraint::Percentage(25),     // Process list
                    ])
                    .split(frame.area());

                let upper_list_layout = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(vec![
                    Constraint::Percentage(36),
                    Constraint::Percentage(25),
                    Constraint::Percentage(39),
                ])
                .split(layout[0]);

                let graphs_layout = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(vec![
                    Constraint::Percentage(33),
                    Constraint::Percentage(33),
                    Constraint::Percentage(34),
                ])
                .split(layout[2]);

                let background = Block::default()
                .style(Style::default().bg(Color::Black));
                frame.render_widget(background, frame.area());

                frame.render_widget(system_info(&sys), upper_list_layout[0]);
                frame.render_widget(usage_info(&sys), upper_list_layout[1]);
                frame.render_widget(cpu_info(&sys), upper_list_layout[2]);
                frame.render_widget(process_list(&sys, &mut state), layout[1]);
                frame.render_widget(cpu_graph(&sys, &mut state), graphs_layout[0]);
            }
        })?;

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
                    KeyCode::Down => state.select_next(total_processes),
                    KeyCode::Up => state.select_previous(),
                    KeyCode::PageDown => state.page_down(total_processes),
                    KeyCode::PageUp => state.page_up(),
                    
                    // Process management
                    KeyCode::Char('h') => state.toggle_help(),
                    KeyCode::Char('k') => {
                        if let Err(e) = send_signal_to_selected_process(&sys, &mut state, Signal::SIGTERM) {
                            eprintln!("Error sending SIGTERM: {}", e);
                        }
                    },
                    KeyCode::Char('u') => {
                        if let Err(e) = send_signal_to_selected_process(&sys, &mut state, Signal::SIGKILL) {
                            eprintln!("Error sending SIGKILL: {}", e);
                        }
                    },                    
                    KeyCode::Char('s') => {
                        if let Err(e) = send_signal_to_selected_process(&sys, &mut state, Signal::SIGSTOP) {
                            eprintln!("Error sending SIGSTOP: {}", e);
                        }
                    },
                    KeyCode::Char('r') => {
                        if let Err(e) = send_signal_to_selected_process(&sys, &mut state, Signal::SIGCONT) {
                            eprintln!("Error sending SIGCONT: {}", e);
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
