--!nonstrict
-- Antarctical bar widget - the current or next event today, in its calendar's colour.
--
--   ● in 12m  Design review  [video]
--
-- The countdown reads "now" while the event is in progress and turns to its start time
-- when it is more than an hour away. The Join button appears from a few minutes before a
-- video call starts until it ends. Click opens the control center's Calendar tab (a
-- binding declared in the manifest), right click opens Antarctical.

local agenda = require("./lib/agenda.luau")

local snapshot = agenda.read(noctalia.state.get(agenda.STATE_KEY))
-- The event the Join button belongs to, as of the last render.
local current = nil

local STATUS_GLYPHS = {
  starting = "calendar-time",
  offline = "calendar-off",
  missing = "calendar-x",
}

-- Today's remaining events, then the first one tomorrow when today is done.
local function tooltipRows(now, tomorrow)
  local rows = {}
  for _, event in ipairs(snapshot.events) do
    if event.declined ~= true and event.endMs > now and event.startMs < tomorrow then
      if #rows == 8 then
        break
      end
      table.insert(rows, { key = agenda.timeRange(event), value = event.title })
    end
  end
  if #rows == 0 then
    table.insert(rows, { key = noctalia.tr("status.idle"), value = "" })
    local later = agenda.pick(snapshot.events, now, agenda.dayStart(now, 2))
    if later ~= nil then
      table.insert(rows, { key = `{noctalia.tr("tooltip.tomorrow")} {agenda.time(later.startMs)}`, value = later.title })
    end
  end
  return rows
end

local function render()
  local now = noctalia.nowMs()
  current = nil

  if snapshot.status ~= "ready" then
    barWidget.setVisible(true)
    barWidget.setTooltip(noctalia.tr("status." .. snapshot.status))
    barWidget.render(ui.glyph({ name = STATUS_GLYPHS[snapshot.status] or "calendar-off", color = "on_surface_variant" }))
    return
  end

  local tomorrow = agenda.dayStart(now, 1)
  local event, phase = agenda.pick(snapshot.events, now, tomorrow)
  barWidget.setTooltip(tooltipRows(now, tomorrow))

  if event == nil then
    barWidget.setVisible(noctalia.getConfig("hide_when_idle") ~= true)
    barWidget.render(ui.glyph({ name = "calendar-check" }))
    return
  end
  barWidget.setVisible(true)
  current = event

  local urgent = phase ~= "next"
  local dot = ui.box({ width = 8, height = 8, radius = 4, fill = event.color ~= "" and event.color or "primary" })
  local countdown = ui.label({
    text = agenda.countdown(event, now),
    fontWeight = "bold",
    color = urgent and "primary" or nil,
    maxLines = 1,
  })

  -- A side bar is one glyph wide: the colour and a short countdown, stacked.
  if barWidget.isVertical() then
    barWidget.render(ui.column({ gap = 2, align = "center" }, { dot, countdown }))
    return
  end

  local children = { dot, countdown }
  if noctalia.getConfig("show_title") ~= false then
    table.insert(children, ui.label({
      text = event.title,
      maxWidth = tonumber(noctalia.getConfig("title_width")) or 220,
      maxLines = 1,
    }))
  end
  if agenda.joinable(event, now) then
    table.insert(children, ui.button({
      key = "join",
      glyph = "video",
      variant = "primary",
      tooltip = noctalia.tr("action.join"),
      onClick = "onJoin",
    }))
  end
  barWidget.render(ui.row({ gap = 6, align = "center" }, children))
end

function onJoin()
  if current ~= nil then
    agenda.browse(current.meetingUrl)
  end
end

function update()
  render()
end

function onRightClick()
  agenda.open(nil, snapshot.status == "ready")
end

function onIpc(event, _payload)
  if event == "join" then
    onJoin()
  end
end

noctalia.state.watch(agenda.STATE_KEY, function(value)
  snapshot = agenda.read(value)
  render()
end)

-- The countdown is in minutes; a short tick keeps "now" and the Join button on time.
noctalia.setUpdateInterval(5000)
render()
