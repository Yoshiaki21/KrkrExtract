//! Command line parsing that reproduces pimgext.py's argparse setup
//! (abbreviated long options, -fq style bundling, "--", error messages,
//! exit status 2 on errors).

use std::io::Write;

use crate::pyfmt::str_repr;
use crate::pypath::NL;

pub const PROG: &str = "pimgext";

const USAGE: &str = "usage: pimgext [-h] [-o DIR] [--no-composite] [-f] [-q] INPUT [INPUT ...]";

const HELP: &str = "\
usage: pimgext [-h] [-o DIR] [--no-composite] [-f] [-q] INPUT [INPUT ...]

Extract and composite .pimg files.

positional arguments:
  INPUT             .pimg file or directory

options:
  -h, --help        show this help message and exit
  -o, --output DIR  parent directory for output
  --no-composite    write raw/ only
  -f, --force       overwrite existing output
  -q, --quiet       suppress warnings
";

#[derive(Default, Debug)]
pub struct Args {
    pub inputs: Vec<String>,
    pub output: Option<String>,
    pub no_composite: bool,
    pub force: bool,
    pub quiet: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Act {
    Help,
    Output,
    NoComposite,
    Force,
    Quiet,
}

impl Act {
    fn nargs(self) -> usize {
        if self == Act::Output {
            1
        } else {
            0
        }
    }

    fn name(self) -> &'static str {
        match self {
            Act::Help => "-h/--help",
            Act::Output => "-o/--output",
            Act::NoComposite => "--no-composite",
            Act::Force => "-f/--force",
            Act::Quiet => "-q/--quiet",
        }
    }
}

/// parser._option_string_actions, in registration order
const OPTIONS: &[(&str, Act)] = &[
    ("-h", Act::Help),
    ("--help", Act::Help),
    ("-o", Act::Output),
    ("--output", Act::Output),
    ("--no-composite", Act::NoComposite),
    ("-f", Act::Force),
    ("--force", Act::Force),
    ("-q", Act::Quiet),
    ("--quiet", Act::Quiet),
];

fn lookup(s: &str) -> Option<Act> {
    OPTIONS.iter().find(|(o, _)| *o == s).map(|&(_, a)| a)
}

/// (action, option_string, sep, explicit_arg)
#[derive(Clone)]
struct OptTuple {
    action: Option<Act>,
    option_string: String,
    sep: Option<String>,
    explicit_arg: Option<String>,
}

fn partition(s: &str) -> (String, Option<String>, Option<String>) {
    match s.split_once('=') {
        Some((a, b)) => (a.to_string(), Some("=".into()), Some(b.to_string())),
        None => (s.to_string(), None, None),
    }
}

fn get_option_tuples(arg: &str) -> Vec<OptTuple> {
    let chars: Vec<char> = arg.chars().collect();
    let mut result = Vec::new();
    let (prefix, sep, explicit) = partition(arg);
    if chars[1] == '-' {
        for &(o, a) in OPTIONS {
            if o.starts_with(&prefix) {
                result.push(OptTuple {
                    action: Some(a),
                    option_string: o.into(),
                    sep: sep.clone(),
                    explicit_arg: explicit.clone(),
                });
            }
        }
    } else {
        let short_prefix: String = chars[..2].iter().collect();
        let short_explicit: String = chars[2..].iter().collect();
        for &(o, a) in OPTIONS {
            if o == short_prefix {
                result.push(OptTuple {
                    action: Some(a),
                    option_string: o.into(),
                    sep: Some(String::new()),
                    explicit_arg: Some(short_explicit.clone()),
                });
            } else if o.starts_with(&prefix) {
                result.push(OptTuple {
                    action: Some(a),
                    option_string: o.into(),
                    sep: sep.clone(),
                    explicit_arg: explicit.clone(),
                });
            }
        }
    }
    result
}

/// re.match(r'-\.?\d', s)
fn looks_negative_number(s: &str) -> bool {
    let mut it = s.chars();
    if it.next() != Some('-') {
        return false;
    }
    match it.next() {
        Some('.') => it.next().is_some_and(char::is_numeric),
        Some(c) => c.is_numeric(),
        None => false,
    }
}

fn parse_optional(arg: &str) -> Option<Vec<OptTuple>> {
    if arg.is_empty() || !arg.starts_with('-') {
        return None;
    }
    if let Some(a) = lookup(arg) {
        return Some(vec![OptTuple { action: Some(a), option_string: arg.into(), sep: None, explicit_arg: None }]);
    }
    if arg.chars().count() == 1 {
        return None;
    }
    let (os, sep, explicit) = partition(arg);
    if sep.is_some() {
        if let Some(a) = lookup(&os) {
            return Some(vec![OptTuple { action: Some(a), option_string: os, sep, explicit_arg: explicit }]);
        }
    }
    let tuples = get_option_tuples(arg);
    if !tuples.is_empty() {
        return Some(tuples);
    }
    if looks_negative_number(arg) || arg.contains(' ') {
        return None;
    }
    Some(vec![OptTuple { action: None, option_string: arg.into(), sep: None, explicit_arg: None }])
}

enum Stop {
    Help,
    Error(String),
}

struct Parser<'a> {
    args: &'a [String],
    pattern: Vec<char>,
    options: Vec<Option<Vec<OptTuple>>>,
    ns: Args,
    inputs_pending: bool,
    extras: Vec<String>,
}

impl Parser<'_> {
    fn take_action(&mut self, act: Act, values: &[String]) -> Result<(), Stop> {
        match act {
            Act::Help => return Err(Stop::Help),
            Act::Output => self.ns.output = Some(values[0].clone()),
            Act::NoComposite => self.ns.no_composite = true,
            Act::Force => self.ns.force = true,
            Act::Quiet => self.ns.quiet = true,
        }
        Ok(())
    }

    /// _match_argument for an optional: '()' or '([A])'
    fn match_argument(act: Act, pattern: &[char]) -> Result<usize, Stop> {
        if act.nargs() == 0 {
            Ok(0)
        } else if pattern.first() == Some(&'A') {
            Ok(1)
        } else {
            Err(Stop::Error(format!("argument {}: expected one argument", act.name())))
        }
    }

    fn consume_optional(&mut self, start: usize) -> Result<usize, Stop> {
        let tuples = self.options[start].clone().unwrap();
        if tuples.len() > 1 {
            let matches: Vec<&str> = tuples.iter().map(|t| t.option_string.as_str()).collect();
            return Err(Stop::Error(format!(
                "ambiguous option: {} could match {}",
                self.args[start],
                matches.join(", ")
            )));
        }
        let OptTuple { mut action, mut option_string, mut sep, mut explicit_arg } = tuples[0].clone();
        let mut action_tuples: Vec<(Act, Vec<String>)> = Vec::new();
        let stop;
        loop {
            let Some(act) = action else {
                self.extras.push(self.args[start].clone());
                return Ok(start + 1);
            };
            if let Some(explicit) = explicit_arg.clone() {
                let arg_count = Self::match_argument(act, &['A'])?;
                let second = option_string.chars().nth(1).unwrap();
                if arg_count == 0 && second != '-' && !explicit.is_empty() {
                    let first = explicit.chars().next().unwrap();
                    if sep.as_deref().is_some_and(|s| !s.is_empty()) || first == '-' {
                        return Err(Stop::Error(format!(
                            "argument {}: ignored explicit argument {}",
                            act.name(),
                            str_repr(&explicit)
                        )));
                    }
                    action_tuples.push((act, Vec::new()));
                    option_string = format!("-{}", first);
                    let rest: String = explicit.chars().skip(1).collect();
                    if let Some(a) = lookup(&option_string) {
                        action = Some(a);
                        if rest.is_empty() {
                            sep = None;
                            explicit_arg = None;
                        } else if let Some(r) = rest.strip_prefix('=') {
                            sep = Some("=".into());
                            explicit_arg = Some(r.to_string());
                        } else {
                            sep = Some(String::new());
                            explicit_arg = Some(rest);
                        }
                    } else {
                        self.extras.push(format!("-{}", explicit));
                        stop = start + 1;
                        break;
                    }
                } else if arg_count == 1 {
                    stop = start + 1;
                    action_tuples.push((act, vec![explicit]));
                    break;
                } else {
                    return Err(Stop::Error(format!(
                        "argument {}: ignored explicit argument {}",
                        act.name(),
                        str_repr(&explicit)
                    )));
                }
            } else {
                let s = start + 1;
                let arg_count = Self::match_argument(act, &self.pattern[s.min(self.pattern.len())..])?;
                stop = s + arg_count;
                action_tuples.push((act, self.args[s..stop].to_vec()));
                break;
            }
        }
        for (act, values) in action_tuples {
            self.take_action(act, &values)?;
        }
        Ok(stop)
    }

    /// consume_positionals: INPUT is '(-*A[A-]*)'
    fn consume_positionals(&mut self, start: usize) -> usize {
        if !self.inputs_pending {
            return start;
        }
        let pat = &self.pattern[start..];
        let mut i = 0;
        while i < pat.len() && pat[i] == '-' {
            i += 1;
        }
        if i >= pat.len() || pat[i] != 'A' {
            return start;
        }
        i += 1;
        while i < pat.len() && (pat[i] == 'A' || pat[i] == '-') {
            i += 1;
        }
        let mut values = self.args[start..start + i].to_vec();
        if pat[..i].contains(&'-') {
            if let Some(k) = values.iter().position(|v| v == "--") {
                values.remove(k);
            }
        }
        self.ns.inputs = values;
        self.inputs_pending = false;
        start + i
    }

    fn run(&mut self) -> Result<(), Stop> {
        let max_opt = self.options.iter().rposition(Option::is_some);
        let mut start = 0usize;
        if let Some(max_opt) = max_opt {
            while start <= max_opt {
                let mut next = start;
                while next <= max_opt && self.options[next].is_none() {
                    next += 1;
                }
                if start != next {
                    let end = self.consume_positionals(start);
                    if end > start {
                        start = end;
                        continue;
                    }
                    start = end;
                }
                if self.options[start].is_none() {
                    self.extras.extend_from_slice(&self.args[start..next]);
                    start = next;
                }
                start = self.consume_optional(start)?;
            }
        }
        let stop = self.consume_positionals(start);
        self.extras.extend_from_slice(&self.args[stop..]);
        if self.inputs_pending {
            return Err(Stop::Error("the following arguments are required: INPUT".into()));
        }
        Ok(())
    }
}

fn print_stdout(text: &str) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(text.replace('\n', NL).as_bytes());
    let _ = out.flush();
}

fn error_exit(msg: &str) -> ! {
    let mut err = std::io::stderr().lock();
    let _ = write!(err, "{}{}{}: error: {}{}", USAGE, NL, PROG, msg, NL);
    let _ = err.flush();
    std::process::exit(2);
}

pub fn parse(args: Vec<String>) -> Args {
    let mut pattern = Vec::new();
    let mut options = Vec::new();
    let mut after_dashdash = false;
    for a in &args {
        if after_dashdash {
            pattern.push('A');
            options.push(None);
        } else if a == "--" {
            after_dashdash = true;
            pattern.push('-');
            options.push(None);
        } else {
            let t = parse_optional(a);
            pattern.push(if t.is_some() { 'O' } else { 'A' });
            options.push(t);
        }
    }
    let mut p = Parser { args: &args, pattern, options, ns: Args::default(), inputs_pending: true, extras: Vec::new() };
    match p.run() {
        Ok(()) => {}
        Err(Stop::Help) => {
            print_stdout(HELP);
            std::process::exit(0);
        }
        Err(Stop::Error(m)) => error_exit(&m),
    }
    if !p.extras.is_empty() {
        error_exit(&format!("unrecognized arguments: {}", p.extras.join(" ")));
    }
    p.ns
}
