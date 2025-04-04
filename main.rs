use sysinfo::{
    Components, Disks, Networks, System,
};
use std::{thread, time::Duration};

use std::io;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{layout::*, widgets::*};

fn system_info(sys: &sysinfo::System) -> Paragraph{
    let sys_info = vec![
        format!("System name:             {:}", System::name().unwrap_or("Unknown".to_string())),
        format!("System kernel version:   {:}", System::kernel_version().unwrap_or("Unknown".to_string())),
        format!("System OS version:       {:}", System::os_version().unwrap_or("Unknown".to_string())),
        format!("System host name:        {:}", System::host_name().unwrap_or("Unknown".to_string())),
        format!("NB CPUs: {}", sys.cpus().len()),
        format!("Total memory: {} MB", sys.total_memory()/ 1048576),
        format!("Used memory : {} MB", sys.used_memory()/ 1048576),
        format!("Total swap  : {} MB", sys.total_swap()/ 1048576),
        format!("Used swap   : {} MB", sys.used_swap()/ 1048576),
    ].join("\n");

    Paragraph::new(sys_info)
    .block(Block::default().title("System Info").borders(Borders::ALL))
}

fn process_list(sys: &sysinfo::System, n:usize) -> List{

    let mut processes: Vec<_> = sys.processes().iter().collect();
    processes.sort_by(|a, b| b.1.cpu_usage().partial_cmp(&a.1.cpu_usage()).unwrap());

        // for (pid, process) in processes.iter().take(5) {
        //     println!("[{pid}] {:?} {:?}", process.name(), process.disk_usage());
        // }

    let proc_list: Vec<ListItem> = processes.iter().take(n)
    .map(|proc|{
        ListItem::new(
            format!( 
            "PID: {} | {:?} | CPU: {:.1}% | MEM: {} MB",
            proc.1.pid(),
            proc.1.name(),
            proc.1.cpu_usage(),
            proc.1.memory() / 1024
        ))
    }).collect();

    List::new(proc_list)
    .block(Block::default().title("Processes").borders(Borders::ALL))
        .highlight_symbol(">")

}



fn main()->io::Result<()>{
    let mut terminal = ratatui::init();

    // creates a new system instance with everything loaded
    let mut sys = System::new_all();
    // refrehsing the system
    sys.refresh_all();

    loop {
        sys.refresh_all();

        terminal.draw(|frame| {
            let layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(10), // System stats
                    Constraint::Min(5),    // Process list
                ])
                .split(frame.area());
        
            frame.render_widget(system_info(&sys), layout[0]);
            frame.render_widget(process_list(&sys, 5), layout[1]);
        })?;

        // handling input => need to update as we handle different types of input
        if crossterm::event::poll(std::time::Duration::from_millis(750))? {
            if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
                if key.code == crossterm::event::KeyCode::Char('q') {
                    break;
                }
            }
        }
    }
    // restore the terminal
     ratatui::restore();
     Ok(())
}

// #[derive(Debug, Default)]
// pub struct App {
//     counter: u8,
//     exit: bool,
// }

// impl App {

//     /// runs the application's main loop until the user quits
//     pub fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
//         while !self.exit {
//             terminal.draw(|frame| self.draw(frame))?;
//             self.handle_events()?;
//         }
//         Ok(())
//     }

//     fn draw(&self, frame: &mut Frame) {
//         frame.render_widget(self, frame.area());
//     }

//         /// updates the application's state based on user input
//     fn handle_events(&mut self) -> io::Result<()> {
//         match event::read()? {
//             // it's important to check that the event is a key press event as
//             // crossterm also emits key release and repeat events on Windows.
//             Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
//                 self.handle_key_event(key_event)
//             }
//             _ => {}
//         };
//         Ok(())
//     }

//     fn handle_key_event(&mut self, key_event: KeyEvent) {
//         match key_event.code {
//             KeyCode::Char('q') => self.exit(),
//             KeyCode::Left => self.decrement_counter(),
//             KeyCode::Right => self.increment_counter(),
//             _ => {}
//         }
//     }

//     fn exit(&mut self) {
//         self.exit = true;
//     }

//     fn increment_counter(&mut self) {
//         self.counter += 1;
//     }

//     fn decrement_counter(&mut self) {
//         self.counter -= 1;
//     }
// }

// impl Widget for &App {
//     fn render(self, area: Rect, buf: &mut Buffer) {
//         let title = Line::from(" Linux Task Manager ".bold());
//         let instructions = Line::from(vec![
//             " Decrement ".into(),
//             "<Left>".blue().bold(),
//             " Increment ".into(),
//             "<Right>".blue().bold(),
//             " Quit ".into(),
//             "<Q> ".blue().bold(),
//         ]);
//         let block = Block::bordered()
//             .title(title.centered())
//             .title_bottom(instructions.centered())
//             .border_set(border::THICK);

//         let counter_text = Text::from(vec![Line::from(vec![
//             "Value: ".into(),
//             self.counter.to_string().yellow(),
//         ])]);

//         Paragraph::new(counter_text)
//             .centered()
//             .block(block)
//             .render(area, buf);
//     }
// }

// fn get_processes(sys: System)->(){

// }

// fn main() -> Result<(), io::Error> {


//     let mut terminal = ratatui::init();
//     let app_result = App::default().run(&mut terminal);
//     ratatui::restore();
//     app_result
    // // initialize the terminal
    // let mut terminal = ratatui::init();
    // loop {
    //     terminal.draw(draw).expect("failed to draw frame");
    //     if matches!(event::read().expect("failed to read event"), Event::Key(_)) {
    //         break;
    //     }
    // }

    // // restore the terminal
    // ratatui::restore();
// } 

// fn draw(frame: &mut Frame) {
//     let text = Text::raw("Hello World!");
//     frame.render_widget(text, frame.area());
// }



// fn main(){

//     // creates a new system instance with everything loaded
//     let mut sys = System::new_all();
//     // refrehsing the system
//     sys.refresh_all();

//     // Do we need errir handling for this part?

//     println!("=> system:");
//     // RAM and swap information:
//     println!("total memory: {} MB", sys.total_memory()/ 1048576);
//     println!("used memory : {} MB", sys.used_memory()/ 1048576);
//     println!("total swap  : {} MB", sys.total_swap()/ 1048576);
//     println!("used swap   : {} MB", sys.used_swap()/ 1048576);

//     // Display system information:
//     println!("System name:             {:}", System::name().unwrap_or("Unknown".to_string()));
//     println!("System kernel version:   {:}", System::kernel_version().unwrap_or("Unknown".to_string()));
//     println!("System OS version:       {:}", System::os_version().unwrap_or("Unknown".to_string()));
//     println!("System host name:        {:}", System::host_name().unwrap_or("Unknown".to_string()));

//     // Number of CPUs:
//     println!("NB CPUs: {}", sys.cpus().len());

//     loop{
//         sys.refresh_all();

//         let mut processes: Vec<_> = sys.processes().iter().collect();
//         processes.sort_by(|a, b| b.1.cpu_usage().partial_cmp(&a.1.cpu_usage()).unwrap());

//         println!("New Refresh");
//         for (pid, process) in processes.iter().take(5) {
//             println!("[{pid}] {:?} {:?}", process.name(), process.disk_usage());
//         }

//         thread::sleep(Duration::from_millis(750));
//     }
// }



// use crossterm::{
//     execute,
//     terminal::{EnterAlternateScreen, LeaveAlternateScreen, enable_raw_mode, disable_raw_mode},
//     cursor::{Hide, Show},
//     event::{poll, read, Event, KeyCode},
//     queue,
//     style::Print,
//     ExecutableCommand, QueueableCommand
// };
// use std::io::{self, Write, stdout};
// use sysinfo::{System, Process};

// fn main() -> io::Result<()> {
//     let mut stdout = stdout();
    
//     // Initialize terminal
//     execute!(stdout, EnterAlternateScreen, Hide)?;
//     enable_raw_mode()?;

//     // Main loop
//     let mut system = System::new_all();
//     loop {
//         system.refresh_all();
        
//         // Queue layout commands
//         queue!(
//             stdout,
//             crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
//             crossterm::cursor::MoveTo(0, 0),
//             Print("=== PROCESS MONITOR ==="),
//             crossterm::cursor::MoveTo(0, 2),
//             Print(format!("{:<8} {:<20} {:<8} {:<8}", "PID", "NAME", "CPU%", "MEM")),
//         )?;

//         // Display processes
//         let mut processes: Vec<_> = system.processes().values().collect();
//         processes.sort_by(|a, b| b.cpu_usage().partial_cmp(&a.cpu_usage()).unwrap());

//         for (i, process) in processes.iter().take(20).enumerate() {
//             queue!(
//                 stdout,
//                 crossterm::cursor::MoveTo(0, 4 + i as u16),
//                 Print(format!(
//                     "{:<8} {:<20?} {:<8.1} {:<8}",
//                     process.pid(),
//                     process.name(),
//                     process.cpu_usage(),
//                     process.memory() / 1024 // Convert to MB
//                 ))
//             )?;
//         }

//         // Handle exit
//         if poll(std::time::Duration::from_millis(750))? {
//             if let Event::Key(event) = read()? {
//                 if event.code == KeyCode::Char('q') {
//                     break;
//                 }
//             }
//         }

//         stdout.flush()?;
//     }

//     // Cleanup
//     execute!(stdout, Show, LeaveAlternateScreen)?;
//     disable_raw_mode()?;
//     Ok(())
// }




// use sysinfo::{
//     Components, Disks, Networks, System, Process
// };

// use std::io::{self, Write};
// use crossterm::{
//     ExecutableCommand, QueueableCommand,
//     terminal, cursor, style::{self, Stylize}
// };

// fn main() -> io::Result<()> {
//   let mut stdout = io::stdout();

//   stdout.execute(terminal::Clear(terminal::ClearType::All))?;

//   for y in 0..40 {
//     for x in 0..150 {
//       if (y == 0 || y == 40 - 1) || (x == 0 || x == 150 - 1) {
//         // in this loop we are more efficient by not flushing the buffer.
//         stdout
//           .queue(cursor::MoveTo(x,y))?
//           .queue(style::PrintStyledContent( "█".magenta()))?;
//       }
//     }
//   }
//   stdout.flush()?;
//   Ok(())
// }

// fn main() {
//     let mut system = System::new_all();
//     system.refresh_all();

//     let mut processes: Vec<_> = system.processes().iter().collect();
//     let mut count = 0;
    
//     // Sort by CPU usage in descending order
//     processes.sort_by(|(_, a), (_, b)| b.cpu_usage().partial_cmp(&a.cpu_usage()).unwrap());

//     // Print sorted processes
//     for (pid, process) in processes {
//         println!("PID: {}, Name: {:?}, CPU Usage: {:.2}%, Status: {}", pid, process.name(), process.cpu_usage(), process.status());
//         count += 1;
//         if count == 10{
//             break;
//         }
//     }
// }
