//! What `decree process`, `daemon` and `tail` print as runs go (docs/reference/cli.md,
//! Output): script output and lines on stdout, notices on stderr, and, on a terminal, one
//! status line kept below them. Every write clears the status line, prints, and draws it
//! again, so nothing is printed over it.

use std::io::{self, IsTerminal, Write};

/// Clear the current terminal line and return to its start.
const CLEAR_LINE: &[u8] = b"\r\x1b[2K";

pub struct Console {
    /// stdout is a terminal: the status line is drawn.
    tty: bool,
    status: Option<String>,
    /// Whether the status line is on screen now.
    drawn: bool,
    /// Script output after its last newline, held back while the status line is drawn.
    partial: Vec<u8>,
}

impl Console {
    /// A console on stdout and stderr, drawing the status line if stdout is a terminal.
    pub fn new() -> Console {
        Console {
            tty: io::stdout().is_terminal(),
            status: None,
            drawn: false,
            partial: Vec::new(),
        }
    }

    /// Show `line` as the status line, or none. Nothing is drawn unless stdout is a terminal.
    pub fn set_status(&mut self, line: Option<String>) -> io::Result<()> {
        if !self.tty || self.status == line {
            return Ok(());
        }
        if line.is_none() {
            self.end_line()?;
        }
        let mut out = io::stdout().lock();
        self.clear(&mut out)?;
        self.status = line;
        self.draw(&mut out)
    }

    /// Print script output. Under a status line only whole lines are printed; the rest
    /// waits for its newline or `end_line`.
    pub fn write_out(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut out = io::stdout().lock();
        if self.status.is_none() {
            out.write_all(bytes)?;
            return out.flush();
        }
        self.partial.extend_from_slice(bytes);
        let Some(end) = self.partial.iter().rposition(|&b| b == b'\n') else {
            return Ok(());
        };
        self.clear(&mut out)?;
        out.write_all(&self.partial[..=end])?;
        self.partial.drain(..=end);
        self.draw(&mut out)
    }

    /// End script output held back without its newline, as a log switches or a run ends.
    pub fn end_line(&mut self) -> io::Result<()> {
        if self.partial.is_empty() {
            return Ok(());
        }
        let mut out = io::stdout().lock();
        self.clear(&mut out)?;
        out.write_all(&self.partial)?;
        out.write_all(b"\n")?;
        self.partial.clear();
        self.draw(&mut out)
    }

    /// Print one line on stdout.
    pub fn line(&mut self, text: &str) -> io::Result<()> {
        self.end_line()?;
        let mut out = io::stdout().lock();
        self.clear(&mut out)?;
        writeln!(out, "{text}")?;
        self.draw(&mut out)
    }

    /// Print one line on stderr.
    pub fn notice(&mut self, text: &str) -> io::Result<()> {
        self.end_line()?;
        let mut out = io::stdout().lock();
        self.clear(&mut out)?;
        eprintln!("{text}");
        self.draw(&mut out)
    }

    fn clear(&mut self, out: &mut impl Write) -> io::Result<()> {
        if self.drawn {
            out.write_all(CLEAR_LINE)?;
            self.drawn = false;
        }
        Ok(())
    }

    fn draw(&mut self, out: &mut impl Write) -> io::Result<()> {
        if let Some(status) = &self.status {
            // One line: a wrapped status line could not be cleared.
            let width = terminal_width().saturating_sub(1).max(1);
            let shown: String = status.chars().take(width).collect();
            out.write_all(shown.as_bytes())?;
            self.drawn = true;
        }
        out.flush()
    }
}

impl Write for Console {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.write_out(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The columns of the terminal on stdout, else 80.
fn terminal_width() -> usize {
    // SAFETY: TIOCGWINSZ writes one `winsize` into `size`.
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    let ok = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) } == 0;
    if ok && size.ws_col > 0 {
        usize::from(size.ws_col)
    } else {
        80
    }
}

/// `45s`, `3m 12s` or `1h 02m`.
pub fn duration(d: std::time::Duration) -> String {
    let s = d.as_secs();
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m {:02}s", s / 60, s % 60),
        _ => format!("{}h {:02}m", s / 3600, s % 3600 / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn durations_read_as_people_write_them() {
        assert_eq!(duration(Duration::from_millis(900)), "0s");
        assert_eq!(duration(Duration::from_secs(45)), "45s");
        assert_eq!(duration(Duration::from_secs(192)), "3m 12s");
        assert_eq!(duration(Duration::from_secs(451)), "7m 31s");
        assert_eq!(duration(Duration::from_secs(3720)), "1h 02m");
    }
}
