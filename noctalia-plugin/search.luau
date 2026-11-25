--!nonstrict
-- Antarctical launcher provider - find an upcoming event by title, place or calendar.
--
-- Selecting an event with a video call that is about to start, or in progress, joins it;
-- any other event opens its day in Antarctical.

local agenda = require("./lib/agenda.luau")

local MAX_RESULTS = 12

local function snapshot()
  return agenda.read(noctalia.state.get(agenda.STATE_KEY))
end

local function message(title)
  return { { id = "", title = title, glyph = "calendar-off" } }
end

local function subtitle(event, now, today)
  local day = noctalia.formatTime("%a %e %b", event.startMs // 1000):gsub("%s+", " ")
  if event.startMs >= today and event.startMs < agenda.dayStart(now, 1) then
    day = noctalia.tr("day.today")
  end
  local parts = { `{day} · {agenda.timeRange(event)}` }
  if event.location ~= "" then
    table.insert(parts, event.location)
  end
  table.insert(parts, noctalia.tr(agenda.joinable(event, now) and "search.join" or "search.open"))
  return table.concat(parts, " · ")
end

function onQuery(query)
  local state = snapshot()
  if state.status ~= "ready" then
    launcher.setResults(query, message(noctalia.tr("status." .. state.status)))
    return
  end

  local text = noctalia.string.trim(query)
  local now = noctalia.nowMs()
  local today = agenda.dayStart(now, 0)
  local matches = {}
  for index, event in ipairs(state.events) do
    if event.endMs > now and event.declined ~= true then
      local score = 0
      if text ~= "" then
        score = noctalia.fuzzyScore(text, `{event.title} {event.location} {event.calendar}`)
      end
      if score ~= nil then
        table.insert(matches, { event = event, index = index, score = score })
      end
    end
  end
  -- Best match first; among equals (and with no query) the soonest.
  table.sort(matches, function(a, b)
    if a.score ~= b.score then
      return a.score > b.score
    end
    return a.index < b.index
  end)

  local results = {}
  for rank, match in ipairs(matches) do
    if rank > MAX_RESULTS then
      break
    end
    local event = match.event
    table.insert(results, {
      id = event.id,
      title = event.title,
      subtitle = subtitle(event, now, today),
      glyph = agenda.joinable(event, now) and "video" or "calendar-event",
      score = MAX_RESULTS - rank,
    })
  end
  if #results == 0 then
    results = message(noctalia.tr(text == "" and "status.none" or "search.none"))
  end
  launcher.setResults(query, results)
end

function onActivate(id)
  local state = snapshot()
  local now = noctalia.nowMs()
  for _, event in ipairs(state.events) do
    if event.id == id then
      if agenda.joinable(event, now) then
        agenda.browse(event.meetingUrl)
      else
        agenda.open(noctalia.formatTime("%Y-%m-%d", math.max(event.startMs, now) // 1000), true)
      end
      return
    end
  end
end
