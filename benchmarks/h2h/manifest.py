"""Run manifest: written next to every result; summarize.py refuses results without one."""
import json, os, platform, subprocess, sys, time

REQUIRED = ("utc", "cpu", "cpus", "mem_kb", "os", "python", "source_digest", "engines", "idle")


def _cpu():
    try:
        for line in open("/proc/cpuinfo"):
            if line.startswith("model name"):
                return line.split(":", 1)[1].strip()
    except OSError:
        pass
    return platform.processor() or "unknown"


def _flags():
    try:
        for line in open("/proc/cpuinfo"):
            if line.startswith("flags"):
                return sorted(f for f in line.split(":", 1)[1].split() if f.startswith(("avx", "sse4", "neon", "fma", "f16c")))
    except OSError:
        pass
    return []


def _mem_kb():
    try:
        return int(open("/proc/meminfo").readline().split()[1])
    except (OSError, ValueError, IndexError):
        return None


def _digest(repo):
    try:
        out = subprocess.run(["bash", "scripts/digest.sh"], cwd=repo, capture_output=True, text=True, timeout=120).stdout
        return out.strip().splitlines()[-1] if out.strip() else "unknown"
    except Exception:
        return "unknown"


def idle_check(limit=0.5):
    """1-minute load average per CPU; None if unavailable. idle=True only when measured and low."""
    try:
        load = float(open("/proc/loadavg").read().split()[0])
    except (OSError, ValueError):
        return {"measured": False, "idle": False, "load_per_cpu": None}
    per = load / (os.cpu_count() or 1)
    return {"measured": True, "idle": per <= limit, "load_per_cpu": round(per, 3)}


def write(out_dir, engines, repo=".", extra=None):
    m = {"utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), "cpu": _cpu(), "simd": _flags(),
         "cpus": os.cpu_count(), "mem_kb": _mem_kb(), "os": platform.platform(), "python": sys.version.split()[0],
         "source_digest": _digest(repo), "engines": engines, "idle": idle_check(), **(extra or {})}
    os.makedirs(out_dir, exist_ok=True)
    json.dump(m, open(os.path.join(out_dir, "manifest.json"), "w"), indent=1)
    return m


def valid(out_dir):
    """(ok, reason). A manifest must exist, have every required key, and say the host was measured idle."""
    p = os.path.join(out_dir, "manifest.json")
    if not os.path.exists(p):
        return False, "no manifest.json"
    m = json.load(open(p))
    miss = [k for k in REQUIRED if k not in m]
    if miss:
        return False, f"missing keys {miss}"
    if not m["idle"].get("idle"):
        return False, "host not measured idle"
    if m["source_digest"] in ("unknown", ""):
        return False, "no source digest"
    return True, "ok"


if __name__ == "__main__":
    # self-check: python manifest.py
    import tempfile
    d = tempfile.mkdtemp()
    assert valid(d) == (False, "no manifest.json")
    json.dump({"utc": 1}, open(os.path.join(d, "manifest.json"), "w"))
    assert not valid(d)[0]
    good = {k: 1 for k in REQUIRED}
    good["idle"] = {"idle": True}
    good["source_digest"] = "abc"
    json.dump(good, open(os.path.join(d, "manifest.json"), "w"))
    assert valid(d) == (True, "ok")
    good["idle"] = {"idle": False}
    json.dump(good, open(os.path.join(d, "manifest.json"), "w"))
    assert valid(d) == (False, "host not measured idle")
    print("manifest self-check ok")
