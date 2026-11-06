# Benchmarks (Persona 2.0, WS-C)

Baseline-first: run this on WebView2 **before** any engine work, then again on each new engine,
so the 2.0 targets in `docs/ENGINE_2_0_PLAN.md` (section 6) are set from real numbers.

```
cargo build --release
powershell -ExecutionPolicy Bypass -File bench\measure.ps1 -Label webview2
```

What it measures (whole process tree of `Persona.exe`, so it works for any engine):

| Metric | Meaning |
|---|---|
| `startupMs` | process start to first main window |
| `privateWorkingSetMB` | memory only this browser uses (the number to compare) |
| `workingSetMB`, `commitMB` | including shared pages / committed memory |
| `marginalPrivateMBPerTab` | (all tabs - idle) / number of tabs |

Each scenario runs in a fresh throwaway `LOCALAPPDATA` with updates off; it never touches your own
profile. Results go to `bench/results/` (git-ignored). Close other browsers first and compare runs
made in the same session; the corpus is live websites, so network and page changes add noise.

Persona now accepts several URLs on the command line (first one is the active tab, the rest open in
the background), which is how the `all` scenario works.

## Not covered yet

- Scroll frame time (smoothness) and input latency.
- JavaScript speed (Speedometer 3 / JetStream): needs a way to read the score back, e.g. through
  the `live_eval` debug-port test on WebView2 and an equivalent hook on other engines.
- A pinned local corpus (saved pages) for repeatable, offline runs.
- CPU use at idle (AGENTS.md says the browser should use none while you read a page).

This script has not been run yet: it was written in a Linux container with no Windows or PowerShell.
Expect to fix small things on the first run.
