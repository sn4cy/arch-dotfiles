#!/bin/bash

hyprctl eval '
local w = hl.get_active_window()
if w == nil then return end

if w.floating then
    hl.dispatch(hl.dsp.window.float({ action = "toggle" }))
else
    hl.dispatch(hl.dsp.window.float({ action = "toggle" }))

    local m = w.monitor
    local width  = math.floor(m.width  / m.scale * 0.99)
    local height = math.floor(m.height / m.scale * 0.94)

    hl.dispatch(hl.dsp.window.resize({ x = width, y = height }))
    hl.dispatch(hl.dsp.window.center())
end
'
