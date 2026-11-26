//! Docker Compose in several worktrees at once: the same file publishes the same host ports, and the second
//! `docker compose up` fails. With `compose = <file>` in `.orcux`, each worktree gets an override that publishes every
//! port of that file on ports of its own block (`ORCUX_PORT` + 50 on), and `COMPOSE_FILE` names both, so plain
//! `docker compose` in its terminals uses them. What the file means is asked to `docker compose config`, never parsed
//! from the file itself.
use crate::*;

/// The first of the worktree's ports given to Compose: those below are for its own servers.
pub const FIRST: u32 = 50;

#[derive(Debug, PartialEq)]
pub struct Port {
    pub service: String,
    pub target: u32,
    pub published: Option<u32>,
    pub protocol: String,
    pub host_ip: Option<String>,
}

/// The ports of every service, out of what `docker compose config` prints (YAML, always in the same shape: one entry
/// per port, ranges already apart).
pub fn ports(config: &str) -> Vec<Port> {
    let (mut out, mut service, mut in_services, mut in_ports) = (vec![], String::new(), false, false);
    for line in config.lines() {
        let indent = line.len() - line.trim_start().len();
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        if indent == 0 {
            in_services = text == "services:";
            continue;
        }
        if !in_services {
            continue;
        }
        if indent == 2 {
            service = text.trim_end_matches(':').trim_matches('"').to_string();
            in_ports = false;
            continue;
        }
        if indent == 4 {
            in_ports = text == "ports:";
            continue;
        }
        if !in_ports || indent < 6 {
            continue;
        }
        let item = text.strip_prefix("- ");
        if item.is_some() {
            out.push(Port { service: service.clone(), target: 0, published: None, protocol: "tcp".into(), host_ip: None });
        }
        let Some((k, v)) = item.unwrap_or(text).split_once(':') else { continue };
        let (k, v) = (k.trim(), v.trim().trim_matches('"'));
        let Some(p) = out.last_mut() else { continue };
        match k {
            "target" => p.target = v.parse().unwrap_or(0),
            "published" => p.published = v.parse().ok(),
            "protocol" => p.protocol = v.into(),
            "host_ip" => p.host_ip = Some(v.into()),
            _ => {}
        }
    }
    out.retain(|p| p.target > 0);
    out
}

/// A variable for each published port, for `.orcux` and the terminals: PORT_<SERVICE>_<TARGET>.
fn var(p: &Port) -> String {
    let service: String = p.service.to_uppercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    format!("PORT_{service}_{}", p.target)
}

/// The override, and the variables of the ports it gives. Every port of a service is written again (`!override`
/// replaces the list): those without a host port as they were.
// ponytail: up to 50 published ports per worktree; more stay as the file says.
pub fn file(id: &str, ports: &[Port], base: u32) -> (String, Vec<(String, String)>) {
    let (mut yaml, mut vars, mut n) = (format!("# orcux: the ports of {id}\nservices:\n"), vec![], 0);
    let mut services: Vec<&str> = ports.iter().map(|p| p.service.as_str()).collect();
    services.dedup();
    for s in services {
        yaml += &format!("  {}:\n    ports: !override\n", quoted(s));
        for p in ports.iter().filter(|p| p.service == s) {
            yaml += &format!("      - target: {}\n        protocol: {}\n", p.target, p.protocol);
            if let Some(ip) = &p.host_ip {
                yaml += &format!("        host_ip: {}\n", quoted(ip));
            }
            if let Some(old) = p.published {
                let port = if n < 100 - FIRST { base + FIRST + n } else { old };
                n += 1;
                yaml += &format!("        published: \"{port}\"\n");
                vars.push((var(p), port.to_string()));
            }
        }
    }
    let header: String = vars.iter().map(|(k, v)| format!("# {k}={v}\n")).collect();
    (format!("{header}{yaml}"), vars)
}

fn quoted(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// COMPOSE_FILE and the port variables of a worktree, with its override written again when the files it comes from
/// changed. Nothing if Docker cannot tell: then Compose works as without orcux.
pub fn env(w: &Wt, files: &[String], known: &[(String, String)]) -> Vec<(String, String)> {
    if files.is_empty() {
        return vec![];
    }
    let dir = w.path();
    let paths: Vec<PathBuf> = files.iter().map(|f| dir.join(f)).collect();
    let out = state_dir().join("compose").join(format!("{}.yml", w.slot));
    let first = format!("# orcux: the ports of {}\n", w.id());
    let age = |p: &Path| fs::metadata(p).and_then(|m| m.modified()).ok();
    let fresh = fs::read_to_string(&out).is_ok_and(|t| t.contains(&first))
        && age(&out).is_some_and(|o| paths.iter().all(|p| age(p).is_none_or(|a| a <= o)));
    if !fresh && let Err(e) = write(w, &paths, known, &out) {
        warn(&e);
        return vec![];
    }
    let text = fs::read_to_string(&out).unwrap_or_default();
    let mut vars: Vec<(String, String)> = text.lines().take_while(|l| l.starts_with("# ") && l.contains('='))
        .filter_map(|l| l[2..].split_once('=')).map(|(k, v)| (k.into(), v.into())).collect();
    let all: Vec<String> = paths.iter().chain([&out]).map(|p| p.display().to_string()).collect();
    vars.push(("COMPOSE_FILE".into(), all.join(":")));
    vars
}

fn write(w: &Wt, paths: &[PathBuf], known: &[(String, String)], out: &Path) -> Res<()> {
    let mut c = Command::new("docker");
    c.args(["compose"]).current_dir(w.path()).env_remove("COMPOSE_FILE").envs(known.iter().map(|(k, v)| (k, v)));
    for p in paths {
        c.arg("-f").arg(p);
    }
    let o = c.arg("config").stdin(std::process::Stdio::null()).output().map_err(|e| format!("docker compose: {e}"))?;
    if !o.status.success() {
        return Err(format!("docker compose config: {}", String::from_utf8_lossy(&o.stderr).trim()));
    }
    let (text, _) = file(&w.id(), &ports(&String::from_utf8_lossy(&o.stdout)), w.port());
    fs::create_dir_all(out.parent().unwrap()).and_then(|_| fs::write(out, text)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_moves_the_ports() {
        let config = "name: x\nservices:\n  db:\n    image: postgres\n    ports:\n      - mode: ingress\n        target: 5432\n        published: \"5432\"\n        protocol: tcp\n  web:\n    ports:\n      - mode: ingress\n        host_ip: 127.0.0.1\n        target: 80\n        published: \"8080\"\n        protocol: tcp\n      - mode: ingress\n        target: 3000\n        protocol: tcp\n      - mode: ingress\n        target: 53\n        published: \"53\"\n        protocol: udp\n  worker:\n    image: busybox\nnetworks:\n  default:\n    name: x_default\n";
        let p = ports(config);
        assert_eq!(p.iter().map(|p| (p.service.as_str(), p.target, p.published)).collect::<Vec<_>>(), [("db", 5432, Some(5432)), ("web", 80, Some(8080)), ("web", 3000, None), ("web", 53, Some(53))]);
        assert_eq!(p[1].host_ip.as_deref(), Some("127.0.0.1"));
        let (yaml, vars) = file("shop/a", &p, 10200);
        assert_eq!(vars, [("PORT_DB_5432".into(), "10250".into()), ("PORT_WEB_80".into(), "10251".into()), ("PORT_WEB_53".into(), "10252".into())]);
        assert!(yaml.contains("  \"web\":\n    ports: !override\n      - target: 80\n        protocol: tcp\n        host_ip: \"127.0.0.1\"\n        published: \"10251\"\n      - target: 3000\n        protocol: tcp\n      - target: 53\n        protocol: udp\n        published: \"10252\""), "{yaml}");
    }
}
