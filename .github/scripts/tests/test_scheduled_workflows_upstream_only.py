"""Scheduled jobs run on trycua/cua only.

A fork that keeps these workflows still receives push, pull_request, and
workflow_call runs. Cron runs skip every job, including jobs that would
otherwise continue after a skipped dependency (`always()`) and reusable
workflow callers (`uses:`), whose `github.event_name` is the caller's event.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[3]
WORKFLOWS = ROOT / ".github/workflows"
UPSTREAM = "trycua/cua"

# Tri-state: True, False, or None when the expression depends on outputs,
# inputs, or status functions other than always().
Tri = bool | None


def load_workflow(path: Path) -> dict:
    text = path.read_text(encoding="utf-8")
    # PyYAML parses a bare `on` key as boolean true.
    text = re.sub(r"^on:\n", '"on":\n', text, count=1, flags=re.M)
    return yaml.safe_load(text)


def scheduled_workflows() -> list[tuple[Path, dict]]:
    found = []
    for path in sorted(WORKFLOWS.glob("*.yml")):
        workflow = load_workflow(path)
        triggers = workflow.get("on") or {}
        if isinstance(triggers, dict) and "schedule" in triggers:
            found.append((path, workflow))
    return found


class _Parser:
    def __init__(self, expression: str, repo: str, event: str) -> None:
        self.expr = expression
        self.repo = repo
        self.event = event
        self.i = 0

    def parse(self) -> Tri:
        value = self._or()
        self._skip()
        if self.i != len(self.expr):
            raise ValueError(f"trailing input at {self.i}: {self.expr[self.i:]!r}")
        return value

    def _skip(self) -> None:
        while self.i < len(self.expr) and self.expr[self.i].isspace():
            self.i += 1

    def _starts(self, text: str) -> bool:
        self._skip()
        return self.expr.startswith(text, self.i)

    def _eat(self, text: str) -> bool:
        if self._starts(text):
            self.i += len(text)
            return True
        return False

    def _or(self) -> Tri:
        value = self._and()
        while self._eat("||"):
            value = _lor(value, self._and())
        return value

    def _and(self) -> Tri:
        value = self._unary()
        while self._eat("&&"):
            value = _land(value, self._unary())
        return value

    def _unary(self) -> Tri:
        if self._eat("!"):
            return _lnot(self._unary())
        return self._comparison()

    def _comparison(self) -> Tri:
        left = self._value()
        if self._eat("=="):
            return self._cmp(left, "==", self._value())
        if self._eat("!="):
            return self._cmp(left, "!=", self._value())
        return _as_bool(left)

    def _value(self):
        self._skip()
        if self._eat("("):
            value = self._or()
            if not self._eat(")"):
                raise ValueError(f"unclosed '(' in {self.expr!r}")
            return value
        if self._starts("'"):
            return ("lit", self._string())
        match = re.match(r"[A-Za-z_*][A-Za-z0-9_.*-]*", self.expr[self.i :])
        if not match:
            raise ValueError(f"expected value at {self.i} in {self.expr!r}")
        name = match.group(0)
        self.i += len(name)
        if self._eat("("):
            args = []
            if not self._starts(")"):
                while True:
                    args.append(self._or())
                    if self._eat(","):
                        continue
                    break
            if not self._eat(")"):
                raise ValueError(f"unclosed call {name}( in {self.expr!r}")
            if name == "always" and not args:
                return True
            return None
        if name == "true":
            return True
        if name == "false":
            return False
        return ("id", name)

    def _string(self) -> str:
        self._skip()
        assert self.expr[self.i] == "'"
        self.i += 1
        chars: list[str] = []
        while self.i < len(self.expr):
            char = self.expr[self.i]
            self.i += 1
            if char == "'":
                if self.i < len(self.expr) and self.expr[self.i] == "'":
                    chars.append("'")
                    self.i += 1
                    continue
                return "".join(chars)
            chars.append(char)
        raise ValueError(f"unterminated string in {self.expr!r}")

    def _cmp(self, left, op: str, right) -> Tri:
        if left == ("id", "github.repository") and right == ("lit", UPSTREAM):
            actual = self.repo == UPSTREAM
        elif left == ("id", "github.event_name") and isinstance(right, tuple) and right[0] == "lit":
            actual = self.event == right[1]
        else:
            return None
        return actual if op == "==" else not actual


def _as_bool(value) -> Tri:
    if value is True or value is False or value is None:
        return value
    if isinstance(value, tuple) and value[0] == "lit":
        return bool(value[1])
    return None


def _land(left: Tri, right: Tri) -> Tri:
    if left is False or right is False:
        return False
    if left is True and right is True:
        return True
    return None


def _lor(left: Tri, right: Tri) -> Tri:
    if left is True or right is True:
        return True
    if left is False and right is False:
        return False
    return None


def _lnot(value: Tri) -> Tri:
    if value is True:
        return False
    if value is False:
        return True
    return None


def evaluate(expression: str, repo: str, event: str) -> Tri:
    text = " ".join(expression.split())
    if text.startswith("${{") and text.endswith("}}"):
        text = text[3:-2].strip()
    return _Parser(text, repo, event).parse()


def job_conditions() -> list[tuple[str, str, str | None]]:
    rows = []
    for path, workflow in scheduled_workflows():
        for name, job in workflow["jobs"].items():
            rows.append((path.name, name, job.get("if")))
    return rows


def test_schedule_guard_keeps_non_schedule_events() -> None:
    guard = "github.repository == 'trycua/cua' || github.event_name != 'schedule'"
    assert evaluate(guard, "fork/example", "schedule") is False
    assert evaluate(guard, UPSTREAM, "schedule") is True
    for event in ("pull_request", "push", "workflow_dispatch", "workflow_call", "workflow_run", "release"):
        assert evaluate(guard, "fork/example", event) is True
        assert evaluate(guard, UPSTREAM, event) is True


def test_schedule_only_job_still_runs_upstream() -> None:
    live = "github.repository == 'trycua/cua' && github.event_name == 'schedule'"
    assert evaluate(live, "fork/example", "schedule") is False
    assert evaluate(live, UPSTREAM, "schedule") is True
    assert evaluate(live, "fork/example", "pull_request") is False
    assert evaluate(live, UPSTREAM, "pull_request") is False


def test_always_continuation_still_skips_a_fork_schedule() -> None:
    summary = "always() && (github.repository == 'trycua/cua' || github.event_name != 'schedule')"
    assert evaluate(summary, "fork/example", "schedule") is False
    assert evaluate(summary, "fork/example", "pull_request") is True
    assert evaluate(summary, UPSTREAM, "schedule") is True


@pytest.mark.parametrize(("workflow", "job", "condition"), job_conditions())
def test_scheduled_job_does_not_run_outside_upstream(workflow: str, job: str, condition: str | None) -> None:
    assert condition, f"{workflow} job {job} has no if, so a fork schedule would run it"
    result = evaluate(condition, "fork/example", "schedule")
    assert result is False, f"{workflow} job {job} can run on a fork schedule: {condition}"
