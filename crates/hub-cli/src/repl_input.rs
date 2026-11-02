//! อ่านบรรทัดใน REPL — พิมพ์ `/` แล้วเลือกคำสั่งด้วย ↑↓ + Enter

use std::io::{self, IsTerminal, Write};
use std::time::Duration;

use crossterm::cursor;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::style::Print;
use crossterm::terminal::{self, Clear, ClearType};
use crossterm::{execute, queue};

use crate::Palette;
use crate::slash::{self, CommandPick};

const PROMPT: &str = "mali_gate> ";

enum Mode {
    Normal,
    SlashSelect { filter: String, selected: usize },
}

struct MenuState {
    lines: u16,
    above: bool,
}

pub fn read_line(paint: &Palette) -> io::Result<Option<String>> {
    if !(io::stdin().is_terminal() && io::stdout().is_terminal()) {
        return read_line_plain(paint);
    }
    read_line_live(paint)
}

fn read_line_plain(paint: &Palette) -> io::Result<Option<String>> {
    print!("{}", paint.heading(PROMPT));
    io::stdout().flush()?;
    let mut line = String::new();
    let n = io::stdin().read_line(&mut line)?;
    if n == 0 {
        return Ok(None);
    }
    Ok(Some(line))
}

#[cfg(windows)]
fn enable_vt_processing() -> io::Result<()> {
    use std::ffi::c_void;
    const STD_OUTPUT_HANDLE: i32 = -11;
    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetStdHandle(nStdHandle: i32) -> *mut c_void;
        fn GetConsoleMode(h: *mut c_void, mode: *mut u32) -> i32;
        fn SetConsoleMode(h: *mut c_void, mode: u32) -> i32;
    }

    unsafe {
        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        if handle.is_null() {
            return Ok(());
        }
        let mut mode: u32 = 0;
        if GetConsoleMode(handle, &mut mode) == 0 {
            return Ok(());
        }
        let _ = SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
    }
    Ok(())
}

#[cfg(not(windows))]
fn enable_vt_processing() -> io::Result<()> {
    Ok(())
}

fn enable_terminal_io() -> io::Result<()> {
    enable_vt_processing()?;
    terminal::enable_raw_mode()
}

fn read_line_live(paint: &Palette) -> io::Result<Option<String>> {
    enable_terminal_io()?;
    let result = read_line_live_inner(paint);
    terminal::disable_raw_mode()?;
    result
}

fn read_line_live_inner(paint: &Palette) -> io::Result<Option<String>> {
    let mut stdout = io::stdout();
    queue!(stdout, cursor::Show)?;
    execute!(stdout, Print("\n"))?;

    let mut buffer = String::new();
    let mut mode = Mode::Normal;
    let mut menu = MenuState {
        lines: 0,
        above: true,
    };
    let mut input_row = cursor::position()?.1;

    redraw_input(paint, &buffer, input_row)?;
    input_row = cursor::position()?.1;

    loop {
        if !event::poll(Duration::from_millis(50))? {
            continue;
        }
        let Event::Key(KeyEvent {
            code,
            modifiers,
            kind: event::KeyEventKind::Press,
            ..
        }) = event::read()?
        else {
            continue;
        };

        if matches!(modifiers, KeyModifiers::CONTROL) {
            match code {
                KeyCode::Char('c') | KeyCode::Char('d') => {
                    clear_menu(&menu, input_row)?;
                    println!();
                    stdout.flush()?;
                    return Ok(None);
                }
                _ => continue,
            }
        }

        let is_slash = matches!(code, KeyCode::Char('/'));

        match (&mut mode, code) {
            (Mode::Normal, _) if is_slash => {
                buffer = "/".to_string();
                mode = Mode::SlashSelect {
                    filter: String::new(),
                    selected: 0,
                };
                redraw_input(paint, &buffer, input_row)?;
                input_row = cursor::position()?.1;
                menu = refresh_select(paint, &mode, input_row, &menu)?;
                restore_input_cursor(input_row, &buffer)?;
            }
            (Mode::Normal, KeyCode::Enter) => {
                clear_menu(&menu, input_row)?;
                println!();
                stdout.flush()?;
                return Ok(if buffer.is_empty() {
                    None
                } else {
                    Some(buffer)
                });
            }
            (Mode::Normal, KeyCode::Backspace) => {
                if buffer.pop().is_some() {
                    redraw_input(paint, &buffer, input_row)?;
                    input_row = cursor::position()?.1;
                }
            }
            (Mode::Normal, KeyCode::Char(ch)) => {
                buffer.push(ch);
                redraw_input(paint, &buffer, input_row)?;
                input_row = cursor::position()?.1;
            }
            (Mode::SlashSelect { filter, selected }, KeyCode::Esc) => {
                buffer.clear();
                *filter = String::new();
                *selected = 0;
                mode = Mode::Normal;
                menu = clear_menu(&menu, input_row)?;
                redraw_input(paint, &buffer, input_row)?;
                input_row = cursor::position()?.1;
            }
            (
                Mode::SlashSelect {
                    filter: _,
                    selected,
                },
                KeyCode::Up,
            ) => {
                if *selected > 0 {
                    *selected -= 1;
                }
                menu = refresh_select(paint, &mode, input_row, &menu)?;
                restore_input_cursor(input_row, &buffer)?;
            }
            (Mode::SlashSelect { filter, selected }, KeyCode::Down) => {
                let max = slash::filtered_count(filter_prefix(filter));
                if max > 0 && *selected + 1 < max {
                    *selected += 1;
                }
                menu = refresh_select(paint, &mode, input_row, &menu)?;
                restore_input_cursor(input_row, &buffer)?;
            }
            (Mode::SlashSelect { filter, selected }, KeyCode::Enter) => {
                let idx = *selected;
                let Some(pick) = slash::pick_filtered(filter_prefix(filter), idx) else {
                    continue;
                };
                match pick {
                    CommandPick::Run(line) => {
                        clear_menu(&menu, input_row)?;
                        println!();
                        stdout.flush()?;
                        return Ok(Some(line));
                    }
                    CommandPick::ContinueTyping(prefix) => {
                        buffer = prefix;
                        mode = Mode::Normal;
                        menu = clear_menu(&menu, input_row)?;
                        redraw_input(paint, &buffer, input_row)?;
                        input_row = cursor::position()?.1;
                    }
                }
            }
            (Mode::SlashSelect { filter, selected }, KeyCode::Tab) => {
                let idx = *selected;
                let Some(name) = slash::tab_complete_name(filter_prefix(filter), idx) else {
                    continue;
                };
                *filter = name.clone();
                buffer = format!("/{name}");
                *selected = 0;
                redraw_input(paint, &buffer, input_row)?;
                input_row = cursor::position()?.1;
                menu = refresh_select(paint, &mode, input_row, &menu)?;
                restore_input_cursor(input_row, &buffer)?;
            }
            (Mode::SlashSelect { filter, selected }, KeyCode::Backspace) => {
                if filter.pop().is_some() {
                    buffer.pop();
                    *selected = 0;
                    redraw_input(paint, &buffer, input_row)?;
                    input_row = cursor::position()?.1;
                    menu = refresh_select(paint, &mode, input_row, &menu)?;
                    restore_input_cursor(input_row, &buffer)?;
                } else {
                    buffer.clear();
                    mode = Mode::Normal;
                    menu = clear_menu(&menu, input_row)?;
                    redraw_input(paint, &buffer, input_row)?;
                    input_row = cursor::position()?.1;
                }
            }
            (Mode::SlashSelect { filter, selected }, KeyCode::Char(ch)) => {
                if ch.is_whitespace() {
                    continue;
                }
                filter.push(ch);
                buffer.push(ch);
                *selected = 0;
                redraw_input(paint, &buffer, input_row)?;
                input_row = cursor::position()?.1;
                menu = refresh_select(paint, &mode, input_row, &menu)?;
                restore_input_cursor(input_row, &buffer)?;
            }
            _ => {}
        }
        stdout.flush()?;
    }
}

fn filter_prefix(filter: &str) -> Option<&str> {
    if filter.is_empty() {
        None
    } else {
        Some(filter)
    }
}

fn refresh_select(
    paint: &Palette,
    mode: &Mode,
    input_row: u16,
    previous: &MenuState,
) -> io::Result<MenuState> {
    clear_menu_rows(previous, input_row)?;
    let Mode::SlashSelect { filter, selected } = mode else {
        return Ok(MenuState {
            lines: 0,
            above: true,
        });
    };
    let (lines, above) =
        slash::draw_select_menu(paint, filter_prefix(filter), *selected, input_row)?;
    Ok(MenuState { lines, above })
}

fn clear_menu(previous: &MenuState, input_row: u16) -> io::Result<MenuState> {
    clear_menu_rows(previous, input_row)?;
    Ok(MenuState {
        lines: 0,
        above: true,
    })
}

fn clear_menu_rows(state: &MenuState, input_row: u16) -> io::Result<()> {
    if state.lines == 0 {
        return Ok(());
    }
    let mut stdout = io::stdout();
    for i in 0..state.lines {
        let row = if state.above {
            input_row.saturating_sub(state.lines - i)
        } else {
            input_row.saturating_add(1 + i)
        };
        queue!(
            stdout,
            cursor::MoveTo(0, row),
            Clear(ClearType::CurrentLine)
        )?;
    }
    stdout.flush()?;
    Ok(())
}

fn redraw_input(paint: &Palette, buffer: &str, input_row: u16) -> io::Result<()> {
    let mut stdout = io::stdout();
    queue!(
        stdout,
        cursor::MoveTo(0, input_row),
        Clear(ClearType::CurrentLine)
    )?;
    // prompt สีได้ แต่ buffer เป็นข้อความธรรมดา — ลดโอกาส wrap พังบน Windows
    print!("{}{}", paint.heading(PROMPT), buffer);
    stdout.flush()?;
    Ok(())
}

fn restore_input_cursor(input_row: u16, buffer: &str) -> io::Result<()> {
    let x = PROMPT.chars().count() as u16 + buffer.chars().count() as u16;
    execute!(io::stdout(), cursor::MoveTo(x, input_row))
}
