use std::io::{self, Write};

use cyberspine_vm::{Backend, RigPolicy, Session, TickRef};

/// One line per tick on stdout, in the image's muscle order.
pub struct Trace {
    names: Vec<String>,
    next: u64,
    dropped: u64,
    broken: bool,
}

impl Trace {
    pub fn new(names: &[String]) -> Self {
        Self {
            names: names.to_vec(),
            next: 0,
            dropped: 0,
            broken: false,
        }
    }

    pub fn header(&mut self) -> io::Result<()> {
        let mut out = io::stdout().lock();
        if let Err(error) = write_header(&mut out, &self.names) {
            return self.fail(error);
        }
        Ok(())
    }

    pub fn drain<B: Backend, P: RigPolicy>(&mut self, session: &Session<B, P>) -> io::Result<()> {
        let mut out = io::stdout().lock();
        for tick in session.trace() {
            if tick.now < self.next {
                continue;
            }
            self.next = tick.now.saturating_add(1);
            if let Err(error) = write_row(&mut out, &tick) {
                return self.fail(error);
            }
        }

        let dropped = session.trace_dropped();
        if dropped > self.dropped {
            self.dropped = dropped;
            eprintln!("warning: the trace ring dropped {dropped} tick(s); the trace is incomplete");
        }
        Ok(())
    }

    pub fn is_broken(&self) -> bool {
        self.broken
    }

    fn fail(&mut self, error: io::Error) -> io::Result<()> {
        if error.kind() == io::ErrorKind::BrokenPipe {
            self.broken = true;
            Ok(())
        } else {
            Err(error)
        }
    }
}

fn write_header(out: &mut impl Write, names: &[String]) -> io::Result<()> {
    write!(out, "tick")?;
    for name in names {
        write!(out, "\t{name}")?;
    }
    writeln!(out)
}

fn write_row(out: &mut impl Write, tick: &TickRef<'_>) -> io::Result<()> {
    write!(out, "{}", tick.now)?;
    for command in tick.commands {
        match command {
            Some(value) => write!(out, "\t{value}")?,
            None => write!(out, "\t-")?,
        }
    }
    writeln!(out)
}
