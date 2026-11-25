--!nonstrict
-- Antarctical service - the only entry that reads Antarctical's event stream.
--
-- `antarctical events --watch` prints the agenda whenever it changes, one JSON object per
-- line, and never exits on its own: it says when Antarctical is not running and keeps
-- looking for it. The service turns those lines into one snapshot on the
-- "antarctical.agenda" state channel, which the bar, panel and launcher read.
--
--   {"type":"begin"} {"type":"event",…}… {"type":"end","events":N}   a complete agenda
--   {"type":"offline"}                                              Antarctical is not running
--   {"type":"ping"}                                                 the stream is alive
--
-- A settings change restarts this runtime (it defines no onConfigChanged), which stops the
-- stream and starts it again with the new command and day count.

local agenda = require("./lib/agenda.luau")

-- The watcher writes at least a ping every 30 s.
local STALE_MS = 100 * 1000
local TICK_MS = 15 * 1000

local published = { status = "starting", events = {} }
-- The events of the agenda being received, between "begin" and "end".
local incoming = nil
-- Lines of a stream that was given up on are ignored.
local generation = 0
local lastLineMs = 0
local lastTickMs = noctalia.nowMs()

local function publish(status, events)
  published = { status = status, events = events or {}, updatedMs = noctalia.nowMs() }
  noctalia.state.set(agenda.STATE_KEY, published)
end

-- Titles and places arrive as typed, sometimes over several lines (a postal address); every
-- entry shows them on one.
local function oneLine(text)
  if type(text) ~= "string" then
    return ""
  end
  return noctalia.string.trim((text:gsub("%s+", " ")))
end

local function onLine(line)
  lastLineMs = noctalia.nowMs()
  local message = noctalia.json.decode(line)
  if type(message) ~= "table" then
    return
  end
  if message.type == "begin" then
    incoming = {}
  elseif message.type == "event" and incoming ~= nil then
    if type(message.startMs) == "number" and type(message.endMs) == "number" then
      message.type = nil
      message.title = oneLine(message.title)
      message.location = oneLine(message.location)
      message.calendar = oneLine(message.calendar)
      table.insert(incoming, message)
    end
  elseif message.type == "end" then
    -- An agenda that lost lines on the way is dropped; the last complete one stays.
    if incoming ~= nil and #incoming == message.events then
      publish("ready", incoming)
    end
    incoming = nil
  elseif message.type == "offline" then
    incoming = nil
    publish("offline")
  end
end

local function shellQuote(text)
  return "'" .. text:gsub("'", "'\\''") .. "'"
end

local function start()
  generation += 1
  incoming = nil
  if not agenda.commandAvailable() then
    lastLineMs = 0
    publish("missing")
    return
  end
  local mine = generation
  local days = math.clamp(math.floor(tonumber(noctalia.getConfig("days")) or 7), 1, 31)
  -- `exec` puts the watcher in the shell's place, so stopping the stream stops the watcher.
  local command = `exec {shellQuote(agenda.command())} events --watch --days {days}`
  local started = noctalia.runStream(command, function(line)
    if mine == generation then
      onLine(line)
    end
  end)
  if started then
    lastLineMs = noctalia.nowMs()
  else
    lastLineMs = 0
    publish("missing")
  end
end

-- A stream that has been silent for too long is dead (the watcher crashed, or the
-- executable went away in an update), and a missing executable may have appeared.
function update()
  local now = noctalia.nowMs()
  -- After a suspend the silence is the sleep, not the stream: give it a fresh allowance.
  if now - lastTickMs > 2 * TICK_MS and lastLineMs ~= 0 then
    lastLineMs = now
  end
  lastTickMs = now
  if lastLineMs == 0 or now - lastLineMs > STALE_MS then
    start()
  end
end

function onIpc(event, _payload)
  if event == "refresh" then
    agenda.refresh()
  elseif event == "open" then
    agenda.open(nil, published.status == "ready")
  end
end

noctalia.setUpdateInterval(TICK_MS)
noctalia.state.set(agenda.STATE_KEY, published)
start()
