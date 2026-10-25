//! Detached guest jobs for `wsm run --detach`, `wsm jobs`, `wsm kill`.
//!
//! A job is a forked process leading its own process group (sshd on lite kills the SSH session's process group
//! on disconnect, so plain `nohup cmd &` dies; BaseSystem's perl has no POSIX module, so `setpgrp`, not `setsid`). State lives on the data disk under `~/.wsm/jobs/<id>/`:
//! `cmd`, `pid`, `log` (stdout+stderr). The guest side is plain sh + perl, present in every base.
use wsm_core::ssh::shell_join;

const DIR: &str = "$HOME/.wsm/jobs";

/// Shell snippet that starts `argv` detached and prints the job id and log path.
pub fn start_script(argv: &[String]) -> String {
    let cmd = shell_join(argv);
    format!(
        r#"set -e
d={DIR}; id=$(date +%y%m%d-%H%M%S)-$$; j=$d/$id; mkdir -p "$j"
cat > "$j/cmd" <<'WSM_JOB_EOF'
{cmd}
WSM_JOB_EOF
perl -e 'my $j = shift @ARGV; exit 0 if fork(); setpgrp(0, 0);
open(STDIN, "</dev/null"); open(STDOUT, ">>$j/log"); open(STDERR, ">&STDOUT");
open(my $f, ">$j/pid"); print $f $$; close $f;
exec(@ARGV) or do {{ print "exec failed: $!\n"; exit 127 }}' "$j" {cmd}
i=0; while [ ! -s "$j/pid" ] && [ $i -lt 50 ]; do sleep 0.1; i=$((i + 1)); done
echo "$id pid $(cat "$j/pid" 2>/dev/null) log $j/log"
"#
    )
}

pub fn list_script() -> String {
    format!(
        r#"d={DIR}
for j in "$d"/*/; do
  [ -d "$j" ] || continue
  id=$(basename "$j"); p=$(cat "$j/pid" 2>/dev/null)
  if [ -n "$p" ] && kill -0 "$p" 2>/dev/null; then s=running; else s=exited; fi
  printf '%-22s %-8s pid %-6s %s\n' "$id" "$s" "$p" "$(head -c 120 "$j/cmd")"
done
exit 0
"#
    )
}

pub fn kill_script(id: &str) -> Result<String, String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(format!("bad job id '{id}'"));
    }
    Ok(format!(
        r#"p=$(cat {DIR}/{id}/pid 2>/dev/null) || {{ echo "no such job: {id}" >&2; exit 1; }}
kill -0 "$p" 2>/dev/null || {{ echo "job {id} is not running"; exit 0; }}
kill -- -"$p" 2>/dev/null || kill "$p"
i=0; while kill -0 "$p" 2>/dev/null && [ $i -lt 30 ]; do sleep 0.1; i=$((i + 1)); done
if kill -0 "$p" 2>/dev/null; then kill -9 -- -"$p" 2>/dev/null || kill -9 "$p"; echo "job {id} killed (SIGKILL)"; else echo "job {id} stopped"; fi
"#
    ))
}
