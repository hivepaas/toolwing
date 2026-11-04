//! `ipcalc`: ipcalc 1.0.3's command line (Red Hat's ipcalc; Debian's
//! ipcalc-ng), its arguments read as glibc's `getopt_long` reads them for
//! its `main`, and its output as it prints it: plain, coloured on a
//! terminal, `--no-decorate`, `VAR=value` for the specific options, or JSON.
//! What it needs the network for, `-h` and `-o` (DNS), is refused: toolwing
//! stays off the network. Geographic information (`-g`) is printed as ipcalc
//! prints it with no database: not at all.

use std::io::{self, Write};

use toolwing_core::Tool as _;
use toolwing_tools::ipcalc::{self as calc, Info};

const VERSION: &str = "1.0.3";

#[derive(Clone, Copy, PartialEq, Eq)]
enum App {
    Version = 1,
    Check = 1 << 1,
    Info = 1 << 2,
    Split = 1 << 3,
    Deaggregate = 1 << 4,
}

// ipcalc.h's flags, by the same bits.
const IPV6: u32 = 1 << 1;
const IPV4: u32 = 1 << 2;
const MODERN: u32 = 1 << 3;
const RESOLVE_IP: u32 = 1 << 4;
const RESOLVE_HOST: u32 = 1 << 5;
const BROADCAST: u32 = 1 << 6;
const NETMASK: u32 = 1 << 7;
const NETWORK: u32 = 1 << 8;
const PREFIX: u32 = 1 << 9;
const MINADDR: u32 = 1 << 10;
const MAXADDR: u32 = 1 << 11;
const ADDRESSES: u32 = 1 << 12;
const ADDRSPACE: u32 = 1 << 13;
const GET_GEOIP: u32 = 1 << 14;
const GEOIP: u32 = (1 << 15) | GET_GEOIP;
const ALL_INFO: u32 = 1 << 16;
const REVERSE: u32 = 1 << 17;
const CLASS_PREFIX: u32 = 1 << 18;
const NO_DECORATE: u32 = 1 << 20;
const ADDRESS: u32 = 1 << 21;
const JSON: u32 = 1 << 22;
const RANDOM: u32 = 1 << 23;
const TO_IGNORE: u32 =
    IPV6 | IPV4 | GET_GEOIP | NO_DECORATE | JSON | CLASS_PREFIX | (1 << 16) | RANDOM;
const ENV_INFO: u32 = NETMASK
    | BROADCAST
    | RESOLVE_IP
    | RESOLVE_HOST
    | ADDRESS
    | REVERSE
    | GEOIP
    | ADDRSPACE
    | ADDRESSES
    | MAXADDR
    | MINADDR
    | PREFIX
    | NETWORK;

const KBLUE: &str = "\x1B[34m";
const KMAG: &str = "\x1B[35m";
const KRESET: &str = "\x1B[0m";

pub fn command() -> clap::Command {
    clap::Command::new("ipcalc")
        .about(toolwing_tools::ipcalc::NetInfo.meta().summary)
        .disable_help_flag(true)
        .arg(
            clap::Arg::new("args")
                .value_name("ARGS")
                .num_args(0..)
                .allow_hyphen_values(true)
                .trailing_var_arg(true),
        )
}

// ---- getopt_long ------------------------------------------------------------

/// ipcalc's options: `S:cr:i46abho:gmnpjsvd:` and its long ones.
const SHORT: &str = "S:cr:i46abho:gmnpjsvd:";

/// (name, takes a value, the option it is).
const LONG: &[(&str, bool, Opt)] = &[
    ("check", false, Opt::Short('c')),
    ("random-private", true, Opt::Short('r')),
    ("split", true, Opt::Short('S')),
    ("deaggregate", true, Opt::Short('d')),
    ("info", false, Opt::Short('i')),
    ("all-info", false, Opt::AllInfo),
    ("ipv4", false, Opt::Short('4')),
    ("ipv6", false, Opt::Short('6')),
    ("address", false, Opt::Short('a')),
    ("broadcast", false, Opt::Short('b')),
    ("hostname", false, Opt::Short('h')),
    ("lookup-host", true, Opt::Short('o')),
    ("reverse-dns", false, Opt::Reverse),
    ("geoinfo", false, Opt::Short('g')),
    ("netmask", false, Opt::Short('m')),
    ("network", false, Opt::Short('n')),
    ("prefix", false, Opt::Short('p')),
    ("class-prefix", false, Opt::ClassPrefix),
    ("minaddr", false, Opt::MinAddr),
    ("maxaddr", false, Opt::MaxAddr),
    ("addresses", false, Opt::Addresses),
    ("addrspace", false, Opt::AddrSpace),
    ("silent", false, Opt::Short('s')),
    ("no-decorate", false, Opt::NoDecorate),
    ("json", false, Opt::Short('j')),
    ("version", false, Opt::Short('v')),
    ("help", false, Opt::Short('?')),
    ("usage", false, Opt::Usage),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Opt {
    Short(char),
    AllInfo,
    MinAddr,
    MaxAddr,
    Addresses,
    AddrSpace,
    Usage,
    Reverse,
    ClassPrefix,
    NoDecorate,
}

/// `getopt_long` for ipcalc's options; '?' for what it refuses.
fn getopt(args: &[String]) -> (Vec<(Opt, Option<String>)>, Vec<String>) {
    let (opts, operands) = crate::getopt::getopt(args, SHORT, Opt::Short, LONG);
    let opts = opts
        .into_iter()
        .map(|g| g.map_or((Opt::Short('?'), None), |g| (g.opt, g.value)))
        .collect();
    (opts, operands)
}

// ---- Output -----------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Chain {
    First,
    Next,
    ArrayFirst,
    ArrayNext,
}

struct Out<'a> {
    w: &'a mut dyn Write,
    flags: u32,
    colors: bool,
    chain: Chain,
}

impl Out<'_> {
    fn json(&self) -> bool {
        self.flags & JSON != 0
    }

    fn plain(&self) -> bool {
        self.flags & NO_DECORATE != 0
    }

    fn start(&mut self) {
        if self.json() {
            let _ = writeln!(self.w, "{{");
        }
        self.chain = Chain::First;
    }

    fn separate(&mut self) {
        if !self.json() {
            let _ = writeln!(self.w);
        }
    }

    fn stop(&mut self) {
        if self.json() {
            let _ = write!(self.w, "\n}}\n");
        }
    }

    fn array_start(&mut self, head: &str, json_head: &str) {
        if self.json() {
            if self.chain == Chain::Next {
                let _ = write!(self.w, ",\n  ");
            }
            let _ = write!(self.w, "  \"{json_head}\":[\n  ");
        } else if !self.plain() {
            let _ = writeln!(self.w, "[{head}]");
        }
        self.chain = Chain::ArrayFirst;
    }

    fn array_stop(&mut self) {
        if self.json() {
            let _ = write!(self.w, "]");
            self.chain = Chain::Next;
        }
    }

    fn colored(&mut self, color: &str, title: &str, value: &str) {
        let _ = write!(self.w, "{title}");
        if self.colors {
            let _ = write!(self.w, "{color}");
        }
        let _ = writeln!(self.w, "{value}");
        if self.colors {
            let _ = write!(self.w, "{KRESET}");
        }
    }

    fn json_line(&mut self, title: Option<&str>, value: &str) {
        match self.chain {
            Chain::ArrayNext => {
                let _ = write!(self.w, ",\n  ");
            }
            Chain::Next => {
                let _ = writeln!(self.w, ",");
            }
            _ => {}
        }
        let _ = write!(self.w, "  ");
        match title {
            Some(t) => {
                let _ = write!(self.w, "\"{t}\":\"{value}\"");
            }
            None => {
                let _ = write!(self.w, "\"{value}\"");
            }
        }
        self.chain = match self.chain {
            Chain::First => Chain::Next,
            Chain::ArrayFirst => Chain::ArrayNext,
            c => c,
        };
    }

    /// `pretty_printf` and `pretty_dist_printf`: "title value" always.
    fn pretty(&mut self, dist: bool, title: &str, json_title: &str, value: &str) {
        if self.json() {
            self.json_line(Some(json_title), value);
        } else if self.plain() {
            let _ = writeln!(self.w, "{title}{value}");
        } else {
            self.colored(if dist { KMAG } else { KBLUE }, title, value);
        }
    }

    /// `default_printf` and `dist_printf`: the value alone when plain.
    fn default(&mut self, dist: bool, title: &str, json_title: Option<&str>, value: &str) {
        if self.json() {
            self.json_line(json_title, value);
        } else if self.plain() {
            let _ = writeln!(self.w, "{value}");
        } else {
            self.colored(if dist { KMAG } else { KBLUE }, title, value);
        }
    }
}

// ---- main ---------------------------------------------------------------------

fn usage(verbose: bool) {
    let text = if verbose {
        "Usage: ipcalc [OPTION...]
  -c, --check                     Validate IP address
  -r, --random-private=PREFIX     Generate a random private IP network using
                                  the supplied prefix or mask.
  -S, --split=PREFIX              Split the provided network using the
                                  provided prefix/netmask
  -d, --deaggregate=IP1-IP2       Deaggregate the provided address range
  -i, --info                      Print information on the provided IP address
                                  (default)
      --all-info                  Print verbose information on the provided IP
                                  address

Specific info options:
      --reverse-dns               Print network in a the reverse DNS format
  -a, --address                   Display IP address
  -b, --broadcast                 Display calculated broadcast address
  -m, --netmask                   Display netmask for IP
  -n, --network                   Display network address
  -p, --prefix                    Display network prefix
      --minaddr                   Display the minimum address in the network
      --maxaddr                   Display the maximum address in the network
      --addresses                 Display the maximum number of addresses in
                                  the network
      --addrspace                 Display the address space the network
                                  resides on
  -h, --hostname                  Show hostname determined via DNS
  -o, --lookup-host=STRING        Show IP as determined via DNS
  -g, --geoinfo                   Show Geographic information about the
                                  provided IP

Other options:
  -4, --ipv4                      Explicitly specify the IPv4 address family
  -6, --ipv6                      Explicitly specify the IPv6 address family
      --class-prefix              When specified the default prefix will be determined
                                  by the IPv4 address class
      --no-decorate               Print only the requested information
  -j, --json                      JSON output
  -s, --silent                    Don't ever display error messages
  -v, --version                   Display program version
  -?, --help                      Show this help message
      --usage                     Display brief usage message
"
    } else {
        "Usage: ipcalc [-46sv?] [-c|--check] [-r|--random-private=STRING] [-i|--info]
        [--all-info] [-4|--ipv4] [-6|--ipv6] [-a|--address] [-b|--broadcast]
        [-h|--hostname] [-o|--lookup-host=STRING] [-g|--geoinfo]
        [-m|--netmask] [-n|--network] [-p|--prefix] [--minaddr] [--maxaddr]
        [--addresses] [--addrspace] [-j|--json] [-s|--silent] [-v|--version]
        [--reverse-dns] [--class-prefix]
        [-?|--help] [--usage]
"
    };
    eprint!("{text}");
}

/// Runs `tw ipcalc`; the exit code. `args` are those after `ipcalc`.
pub fn main(args: &[String]) -> u8 {
    let stdout = crate::stdout();
    let colors = stdout.is_terminal();
    let mut w = io::BufWriter::new(stdout);
    let code = run(args, &mut w, colors);
    let _ = w.flush();
    code
}

struct Run {
    silent: bool,
}

impl Run {
    /// A message, unless `-s`; then 1.
    fn fail(&self, message: &str) -> u8 {
        if !self.silent {
            eprintln!("ipcalc: {message}");
        }
        1
    }
}

fn run(args: &[String], w: &mut dyn Write, colors: bool) -> u8 {
    let (opts, operands) = getopt(args);
    let mut flags = 0u32;
    let mut app = 0u32;
    let (mut split_text, mut ip_text, mut random_text, mut hostname) = (
        None::<String>,
        None::<String>,
        None::<String>,
        None::<String>,
    );
    let mut r = Run { silent: false };
    for (opt, value) in opts {
        let v = value.unwrap_or_default();
        match opt {
            Opt::Short('c') => app |= App::Check as u32,
            Opt::Short('S') => {
                app |= App::Split as u32;
                split_text = Some(v);
            }
            Opt::Short('d') => {
                app |= App::Deaggregate as u32;
                ip_text = Some(v);
            }
            Opt::Short('r') => {
                app |= App::Info as u32;
                flags |= RANDOM;
                random_text = Some(v);
            }
            Opt::Short('i') => {
                app |= App::Info as u32;
                flags |= MODERN;
            }
            Opt::AllInfo => {
                app |= App::Info as u32;
                flags |= ALL_INFO | MODERN;
            }
            Opt::ClassPrefix => flags |= CLASS_PREFIX,
            Opt::Reverse => {
                app |= App::Info as u32;
                flags |= REVERSE;
            }
            Opt::Short('4') => flags |= IPV4,
            Opt::Short('6') => flags |= IPV6,
            Opt::Short(c @ ('a' | 'b' | 'h' | 'g' | 'm' | 'n' | 'p')) => {
                app |= App::Info as u32;
                flags |= match c {
                    'a' => ADDRESS,
                    'b' => BROADCAST,
                    'h' => RESOLVE_HOST,
                    'g' => GEOIP,
                    'm' => NETMASK,
                    'n' => NETWORK,
                    _ => PREFIX,
                };
            }
            Opt::Short('o') => {
                app |= App::Info as u32;
                flags |= RESOLVE_IP;
                hostname = Some(v);
            }
            Opt::MinAddr | Opt::MaxAddr | Opt::Addresses | Opt::AddrSpace => {
                app |= App::Info as u32;
                flags |= match opt {
                    Opt::MinAddr => MINADDR,
                    Opt::MaxAddr => MAXADDR,
                    Opt::Addresses => ADDRESSES,
                    _ => ADDRSPACE,
                };
            }
            Opt::NoDecorate => flags |= NO_DECORATE,
            Opt::Short('j') => flags |= JSON,
            Opt::Short('s') => r.silent = true,
            Opt::Short('v') => app |= App::Version as u32,
            // --usage and anything getopt refused: the usage, and 0.
            Opt::Usage => {
                usage(false);
                return 0;
            }
            _ => {
                usage(true);
                return 0;
            }
        }
    }
    let mut operands = operands.into_iter();
    let mut extra = None;
    if let Some(first) = operands.next() {
        if ip_text.is_some() {
            return r.fail("superfluous option given");
        }
        ip_text = Some(first);
        extra = operands.next();
    }
    if flags & JSON != 0 && flags & NO_DECORATE != 0 {
        flags &= !NO_DECORATE;
    }
    if flags & !TO_IGNORE == 0 {
        flags |= MODERN;
    }
    if flags & !ENV_INFO != 0 && flags & JSON != 0 {
        flags |= MODERN;
    }
    if app.count_ones() > 1 {
        return r.fail("you cannot mix these options");
    }
    if flags & IPV6 != 0 && flags & IPV4 != 0 {
        return r.fail("you cannot specify both IPv4 and IPv6");
    }
    if flags & IPV4 == 0 && ip_text.as_deref().is_some_and(|s| s.contains(':')) {
        flags |= IPV6;
    }
    let mut out = Out {
        w,
        flags,
        colors: false,
        chain: Chain::First,
    };
    if app == App::Version as u32 {
        let _ = writeln!(
            out.w,
            "tw ipcalc {} (ipcalc {VERSION})",
            env!("CARGO_PKG_VERSION")
        );
        return 0;
    }
    if app == App::Deaggregate as u32 {
        let range = ip_text.unwrap_or_default();
        let nets = match calc::deaggregate(&range, flags & IPV6 != 0) {
            Ok(n) => n,
            Err(e) => return r.fail(&e),
        };
        out.start();
        out.array_start("Deaggregated networks", "DEAGGREGATEDNETWORK");
        for net in nets {
            out.default(false, "Network:\t", None, &net);
        }
        out.array_stop();
        out.stop();
        return 0;
    }
    if hostname.is_some() || flags & RESOLVE_HOST != 0 {
        return r.fail("names are not looked up: toolwing stays off the network");
    }
    let mut v6 = flags & IPV6 != 0;
    if let Some(text) = &random_text {
        if let Some(ip) = &ip_text {
            return r.fail(&format!("provided superfluous parameter '{ip}'"));
        }
        let Some(prefix) = calc::str_to_prefix(&mut v6, text, true) else {
            return r.fail(&format!("bad {} prefix: {text}", family(v6)));
        };
        match calc::random_network(prefix, v6) {
            Some(net) => ip_text = Some(net),
            None => {
                return r.fail(&format!("cannot generate network with prefix: {prefix}"));
            }
        }
        if v6 {
            flags |= IPV6;
        }
    }
    let Some(mut ip) = ip_text else {
        if !r.silent {
            eprintln!("ipcalc: ip address expected");
            usage(true);
        }
        return 1;
    };
    let mut prefix_text = None;
    if let Some(extra) = extra {
        if !v6 {
            prefix_text = Some(extra);
        } else {
            if !r.silent {
                eprintln!("ipcalc: unexpected argument: {extra}");
                usage(true);
            }
            return 1;
        }
    }
    if prefix_text.is_none()
        && let Some((addr, p)) = ip.clone().split_once('/')
    {
        prefix_text = Some(p.to_owned());
        ip = addr.to_owned();
    }
    let mut prefix = None;
    if let Some(p) = &prefix_text {
        match calc::str_to_prefix(&mut v6, p, false) {
            Some(n) => prefix = Some(n),
            None => return r.fail(&format!("bad {} prefix: {p}", family(v6))),
        }
    }
    let info = if v6 {
        calc::v6_info(&ip, prefix)
    } else {
        calc::v4_info(&ip, prefix, flags & CLASS_PREFIX != 0)
    };
    let info = match info {
        Ok(i) => i,
        Err(e) => return r.fail(&e),
    };
    if app == App::Split as u32 {
        let text = split_text.unwrap_or_default();
        let Some(p) = calc::str_to_prefix(&mut v6, &text, true) else {
            return r.fail(&format!("bad {} prefix: {text}", family(v6)));
        };
        // A prefix past 32 makes an IPv4 network's split IPv6's, which
        // cannot read the IPv4 network back.
        if v6 && !info.v6 {
            return r.fail(&format!("bad IPv6 address: {}", info.network));
        }
        let nets = match calc::split(&info, p) {
            Ok(n) => n,
            Err(e) => {
                if !r.silent {
                    eprintln!("{e}");
                }
                return 1;
            }
        };
        let plain = flags & NO_DECORATE != 0 && flags & JSON == 0;
        out.start();
        out.array_start("Split networks", "SPLITNETWORK");
        let mut count = 0u32;
        for net in nets {
            if plain {
                let _ = writeln!(out.w, "{net}");
            } else {
                out.default(false, "Network:\t", None, &net);
            }
            count = count.wrapping_add(1);
        }
        out.array_stop();
        if !plain {
            let hosts = if info.v6 {
                // The reference's buffer holds 31 characters.
                calc::v6_hosts(p).chars().take(31).collect()
            } else {
                calc::v4_hosts(p)
            };
            out.default(true, "\nTotal:  \t", Some("NETS"), &count.to_string());
            out.default(true, "Hosts/Net:\t", Some("ADDRESSES"), &hosts);
        }
        out.stop();
        return 0;
    }
    if app == App::Check as u32 {
        return 0;
    }
    out.colors = colors;
    show(&mut out, &info, flags, random_text.is_some());
    0
}

fn family(v6: bool) -> &'static str {
    if v6 { "IPv6" } else { "IPv4" }
}

/// What ipcalc prints for an address: its modern view, or `VAR=value`
/// lines for the options that name them.
fn show(out: &mut Out, info: &Info, flags: u32, random: bool) {
    let json = flags & JSON != 0;
    if flags & MODERN != 0 {
        let single = (info.v6 && info.prefix == 128) || (!info.v6 && info.prefix == 32);
        out.start();
        if (!random || single) && (single || info.network != info.ip) {
            if let Some(full) = &info.expanded_ip {
                out.pretty(false, "Full Address:\t", "FULLADDRESS", full);
            }
            out.pretty(false, "Address:\t", "ADDRESS", &info.ip);
        }
        if !single || json {
            if !json {
                if let Some(full) = &info.expanded_network {
                    out.pretty(
                        false,
                        "Full Network:\t",
                        "FULLNETWORK",
                        &format!("{full}/{}", info.prefix),
                    );
                }
                out.pretty(
                    false,
                    "Network:\t",
                    "NETWORK",
                    &format!("{}/{}", info.network, info.prefix),
                );
                out.pretty(
                    false,
                    "Netmask:\t",
                    "NETMASK",
                    &format!("{} = {}", info.netmask, info.prefix),
                );
            } else {
                if let Some(full) = &info.expanded_network {
                    out.pretty(false, "Full Network:\t", "FULLNETWORK", full);
                }
                out.pretty(false, "Network:\t", "NETWORK", &info.network);
                out.pretty(false, "Netmask:\t", "NETMASK", &info.netmask);
                out.pretty(false, "Prefix:\t", "PREFIX", &info.prefix.to_string());
            }
            if let Some(b) = &info.broadcast {
                out.pretty(false, "Broadcast:\t", "BROADCAST", b);
            }
        }
        if flags & ALL_INFO != 0
            && let Some(r) = &info.reverse_dns
        {
            out.pretty(false, "Reverse DNS:\t", "REVERSEDNS", r);
        }
        if !single || json {
            out.separate();
            out.pretty(true, "Address space:\t", "ADDRSPACE", info.space);
            if flags & ALL_INFO != 0
                && let Some(c) = info.class
            {
                out.pretty(true, "Address class:\t", "ADDRCLASS", c);
            }
            out.pretty(false, "HostMin:\t", "MINADDR", &info.hostmin);
            out.pretty(false, "HostMax:\t", "MAXADDR", &info.hostmax);
            if info.v6 && info.prefix < 112 && !json {
                let hosts = format!("2^({}) = {}", 128 - info.prefix, info.hosts);
                out.pretty(false, "Hosts/Net:\t", "ADDRESSES", &hosts);
            } else {
                out.pretty(false, "Hosts/Net:\t", "ADDRESSES", &info.hosts);
            }
        } else {
            out.pretty(true, "Address space:\t", "ADDRSPACE", info.space);
            if flags & ALL_INFO != 0
                && let Some(c) = info.class
            {
                out.pretty(true, "Address class:\t", "ADDRCLASS", c);
            }
        }
        out.stop();
        return;
    }
    let decorate = flags & NO_DECORATE == 0;
    let mut line = |name: &str, value: &str| {
        if decorate {
            let _ = write!(out.w, "{name}=");
        }
        let _ = writeln!(out.w, "{value}");
    };
    let quoted = |v: &str| {
        if v.contains(' ') {
            format!("\"{v}\"")
        } else {
            v.to_owned()
        }
    };
    if flags & ADDRESS != 0 {
        line("ADDRESS", &info.ip);
    }
    if flags & NETMASK != 0 {
        line("NETMASK", &info.netmask);
    }
    if flags & PREFIX != 0 {
        line("PREFIX", &info.prefix.to_string());
    }
    if flags & BROADCAST != 0 && !info.v6 {
        line("BROADCAST", info.broadcast.as_deref().unwrap_or("(null)"));
    }
    if flags & NETWORK != 0 {
        line("NETWORK", &info.network);
    }
    // A network with no reverse zone (IPv4 under /8, IPv6 not a whole
    // nibble): the reference crashes printing it; this leaves the line out.
    if flags & REVERSE != 0
        && let Some(r) = &info.reverse_dns
    {
        line("REVERSEDNS", r);
    }
    if flags & MINADDR != 0 {
        line("MINADDR", &info.hostmin);
    }
    if flags & MAXADDR != 0 {
        line("MAXADDR", &info.hostmax);
    }
    if flags & ADDRSPACE != 0 {
        line("ADDRSPACE", &quoted(info.space));
    }
    if flags & ADDRESSES != 0 && !info.hosts.is_empty() {
        line("ADDRESSES", &quoted(&info.hosts));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| (*a).to_owned()).collect()
    }

    /// What `ipcalc ARGS` prints, as run here.
    fn printed(args: &[String]) -> (u8, String) {
        let mut out = Vec::new();
        let code = run(args, &mut out, false);
        (code, String::from_utf8_lossy(&out).into_owned())
    }

    /// The app's "copy as command" for ipcalc prints what the app shows.
    #[test]
    fn copied_commands_run_what_the_app_ran() {
        use toolwing_core::{Ctx, Output, Tool, Value, Values};
        use toolwing_tools::{command, ipcalc};
        let lines = |o: Output| match o {
            Output::Lines(l) => l,
            _ => Vec::new(),
        };
        let tail = |args: Vec<String>| args[1..].to_vec();

        let mut v = Values::new();
        v.set("prefix", Value::Text("28".into()));
        let mut args = tail(ipcalc::NetSplit.command_args(&v).unwrap_or_default());
        args.push("192.168.1.0/26".into());
        let app = ipcalc::NetSplit
            .run(&Ctx::new(), b"192.168.1.0/26", &v)
            .map(lines);
        let (code, text) = printed(&args);
        let cli: Vec<String> = text
            .lines()
            .filter_map(|l| l.strip_prefix("Network:\t").map(str::to_owned))
            .collect();
        assert_eq!((code, Ok(cli)), (0, app));
        assert_eq!(
            command::shell_line(&ipcalc::NetSplit, &v, "192.168.1.0/26"),
            "tw ipcalc -S 28 192.168.1.0/26"
        );

        let v = Values::new();
        let mut args = tail(ipcalc::NetDeaggregate.command_args(&v).unwrap_or_default());
        args.push("10.0.0.5-10.0.0.70".into());
        let app = ipcalc::NetDeaggregate
            .run(&Ctx::new(), b"10.0.0.5-10.0.0.70", &v)
            .map(lines);
        let (code, text) = printed(&args);
        let cli: Vec<String> = text
            .lines()
            .filter_map(|l| l.strip_prefix("Network:\t").map(str::to_owned))
            .collect();
        assert_eq!((code, Ok(cli)), (0, app));

        let mut args = tail(ipcalc::NetInfo.command_args(&v).unwrap_or_default());
        args.extend(["-j".to_owned(), "2001:db8::1/64".to_owned()]);
        let (code, text) = printed(&args);
        assert_eq!(code, 0);
        let fields = match ipcalc::NetInfo.run(&Ctx::new(), b"2001:db8::1/64", &v) {
            Ok(Output::Fields(f)) => f,
            other => panic!("{other:?}"),
        };
        for (json, key) in [
            ("MINADDR", "min_address"),
            ("MAXADDR", "max_address"),
            ("ADDRSPACE", "address_space"),
        ] {
            let value = fields
                .iter()
                .find(|f| f.key == key)
                .map(|f| f.value.clone())
                .unwrap_or_default();
            assert!(
                text.contains(&format!("\"{json}\":\"{value}\"")),
                "{json}: {text}"
            );
        }
    }

    #[test]
    fn options_read_as_getopt_long_reads_them() {
        let (o, rest) = getopt(&s(&[
            "10.0.0.1/8",
            "-nb",
            "-S24",
            "--json",
            "x",
            "--",
            "-a",
        ]));
        assert_eq!(
            o,
            [
                (Opt::Short('n'), None),
                (Opt::Short('b'), None),
                (Opt::Short('S'), Some("24".into())),
                (Opt::Short('j'), None),
            ]
        );
        assert_eq!(rest, ["10.0.0.1/8", "x", "-a"]);
        // Unique prefixes; an ambiguous one, an unknown one, a value missing.
        assert_eq!(getopt(&s(&["--no-d"])).0, [(Opt::NoDecorate, None)]);
        assert_eq!(getopt(&s(&["--net"])).0, [(Opt::Short('?'), None)]);
        assert_eq!(getopt(&s(&["-x"])).0, [(Opt::Short('?'), None)]);
        assert_eq!(getopt(&s(&["-S"])).0, [(Opt::Short('?'), None)]);
        assert_eq!(
            getopt(&s(&["--split", "-4"])).0,
            [(Opt::Short('S'), Some("-4".into()))]
        );
        assert_eq!(getopt(&s(&["--json=1"])).0, [(Opt::Short('?'), None)]);
        assert_eq!(getopt(&s(&["-"])).1, ["-"]);
    }
}
