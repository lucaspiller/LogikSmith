# Schedules

Schedules are block-local triggers. Fixed schedules use a local time, interval
schedules use a UTC phase, and astronomical schedules use a solar event plus a
signed offset. `weekdays` is the only calendar condition in a schedule rule.

An astronomical schedule is not clamped to an `earliest` or `latest` time. Use
separate schedules when an automation needs separate trigger times, and use
Lua for conditions on the captured solar values.

For example, trigger 15 minutes before sunset but emit only when the raw local
sunset is no later than 21:15:

```toml
[[blocks.schedules]]
name = "sunset_check"
enabled = true
kind = "astronomical"
anchor = "sunset"
offset = "-15m"
```

```lua
function handle(event, input, meta, state, ctx)
    if event.type == "schedule"
        and event.schedule == "sunset_check"
        and ctx.sun.sunset <= "21:15" then
        return { outputs = { test_light = true } }
    end
end
```

`DateTimeValue <` and `DateTimeValue <=` accept canonical local-time strings in
`HH:MM` or `HH:MM:SS` form. They compare only the local hour, minute, and
second, ignoring the date; `DateTimeValue`-to-`DateTimeValue` comparisons keep
their instant semantics. An unavailable value compares false. Malformed time
strings produce a contained Lua runtime error.

For example, a morning condition can use `ctx.now < "06:00"`. The comparison
syntax is intentionally limited to ordering on the left-hand `DateTimeValue`;
there is no broad time-construction API.
