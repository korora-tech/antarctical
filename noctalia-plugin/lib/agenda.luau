--!nonstrict
-- What every entry needs to read the agenda the service publishes: which event matters
-- right now, how to say when it is, and how to act on one.
--
-- An event is one line of `antarctical events --watch`:
--   { id, title, calendar, color, startMs, endMs, allDay, location, url, meetingUrl,
--     tentative, declined }
-- Times are wall-clock milliseconds, the same base as noctalia.nowMs(). All-day events
-- start and end at local midnight.

local M = {}

M.STATE_KEY = "antarctical.agenda"

local MINUTE = 60 * 1000
local HOUR = 60 * MINUTE
local DAY_SECONDS = 86400

-- The agenda as last published, or a placeholder until the service has spoken.
-- status: "starting" | "ready" | "offline" | "missing".
function M.read(value)
  if type(value) == "table" and type(value.events) == "table" then
    return value
  end
  return { status = "starting", events = {} }
end

function M.joinLeadMs()
  return (tonumber(noctalia.getConfig("join_lead_minutes")) or 5) * MINUTE
end

-- Events that can be "the next meeting": timed, and not declined.
local function eligible(event)
  return event.allDay ~= true and event.declined ~= true
end

-- The event to put in front of the user at `now`, and why:
--   "soon"  starts within the join lead; it wins so it can be joined on time
--   "now"   in progress (the one that started last, when meetings overlap)
--   "next"  the next to start
-- Only events starting before `horizon` count as next. Returns nil when nothing is left.
function M.pick(events, now, horizon)
  local lead = M.joinLeadMs()
  local soon, ongoing, nextUp = nil, nil, nil
  for _, event in ipairs(events) do
    if eligible(event) and event.endMs > now then
      if event.startMs <= now then
        if ongoing == nil or event.startMs > ongoing.startMs then
          ongoing = event
        end
      elseif event.startMs < horizon then
        if event.startMs - now <= lead and (soon == nil or event.startMs < soon.startMs) then
          soon = event
        end
        if nextUp == nil or event.startMs < nextUp.startMs then
          nextUp = event
        end
      end
    end
  end
  if soon ~= nil then
    return soon, "soon"
  elseif ongoing ~= nil then
    return ongoing, "now"
  elseif nextUp ~= nil then
    return nextUp, "next"
  end
  return nil, nil
end

-- True while an event's video call is worth offering: from the join lead before it starts
-- until it ends.
function M.joinable(event, now)
  return event.meetingUrl ~= "" and event.startMs - now <= M.joinLeadMs() and event.endMs > now
end

function M.time(ms)
  return noctalia.formatTime(noctalia.timeFormat(), ms // 1000)
end

-- "now", "in 12m", "in 1h 5m", or the start time once it is more than `clockAfter` away.
function M.countdown(event, now, clockAfter)
  local untilStart = event.startMs - now
  if untilStart <= 0 then
    return noctalia.tr("when.now")
  end
  if untilStart >= (clockAfter or HOUR) then
    return M.time(event.startMs)
  end
  local minutes = math.ceil(untilStart / MINUTE)
  if minutes < 60 then
    return noctalia.tr("when.minutes", { m = minutes })
  end
  local hours, rest = minutes // 60, minutes % 60
  if rest == 0 then
    return noctalia.tr("when.hours", { h = hours })
  end
  return noctalia.tr("when.hours_minutes", { h = hours, m = rest })
end

-- "09:30 – 10:00", or "All day".
function M.timeRange(event)
  if event.allDay == true then
    return noctalia.tr("when.all_day")
  end
  return `{M.time(event.startMs)} – {M.time(event.endMs)}`
end

-- Epoch seconds of local midnight for the day containing `at` (epoch seconds).
local function localMidnight(at)
  local h = tonumber(noctalia.formatTime("%H", at)) or 0
  local m = tonumber(noctalia.formatTime("%M", at)) or 0
  local s = tonumber(noctalia.formatTime("%S", at)) or 0
  return at - h * 3600 - m * 60 - s
end

-- Local midnight, in ms, of the day `offset` days after the one containing `now`. Each day
-- is found from the previous noon and re-anchored to its own midnight, so a DST change in
-- between does not shift it.
function M.dayStart(now, offset)
  local today = localMidnight(now // 1000)
  if offset == 0 then
    return today * 1000
  end
  return localMidnight(today + offset * DAY_SECONDS + 12 * 3600) * 1000
end

-- ── Actions ──────────────────────────────────────────────────────────────────

local function command()
  local configured = noctalia.getConfig("command")
  if type(configured) == "string" and noctalia.string.trim(configured) ~= "" then
    return noctalia.string.trim(configured)
  end
  return "antarctical"
end
M.command = command

-- Whether the configured executable can be run at all.
function M.commandAvailable()
  local bin = command()
  if bin:find("/", 1, true) then
    return noctalia.fileExists(bin)
  end
  return noctalia.commandExists(bin)
end

-- Brings Antarctical's window forward, on `date` ("YYYY-MM-DD") when given. Launching it
-- again is how a stopped Antarctical starts and how a running one shows its window; only a
-- running one understands `show --date`.
function M.open(date, running)
  if date ~= nil and running then
    noctalia.runAsync({ command(), "show", "--date", date }, function(result)
      if result.exitCode ~= 0 then
        noctalia.runAsync({ command() })
      end
    end)
  else
    noctalia.runAsync({ command() })
  end
end

function M.refresh()
  noctalia.runAsync({ command(), "refresh" }, function(result)
    if result.exitCode ~= 0 then
      noctalia.notifyError(noctalia.tr("notify.failed"), noctalia.string.trim(result.stderr))
    end
  end)
end

-- Opens a web address in the browser. The stream only ever carries http(s) addresses; the
-- check here keeps that true whatever published the state.
function M.browse(url)
  if type(url) ~= "string" or not (url:find("^https://") or url:find("^http://")) then
    return false
  end
  return noctalia.runAsync({ "xdg-open", url }, function(result)
    if result.exitCode ~= 0 then
      noctalia.notifyError(noctalia.tr("notify.open_failed"), noctalia.string.trim(result.stderr))
    end
  end)
end

return M
