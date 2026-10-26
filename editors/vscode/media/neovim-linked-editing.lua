-- VSCode Neovim syncs a native normal-mode edit and its VS Code linked-editing
-- companion separately. Join only that verified companion at the sync boundary.
local M = { states = {}, attached = {}, enabled = true }
local api = vim.api
local internal = require('vscode.internal')
local original = internal.handle_changes
local namespace = api.nvim_create_namespace('texquill.linked-editing')

local function names(buf)
  local result, ranges = {}, {}
  for id = 1, 2 do
    local mark = api.nvim_buf_get_extmark_by_id(buf, namespace, id, { details = true })
    if #mark == 0 or mark[1] ~= mark[3].end_row then return end
    ranges[id] = { row = mark[1], first = mark[2], last = mark[3].end_col }
    result[id] = api.nvim_buf_get_text(buf, mark[1], mark[2], mark[3].end_row, mark[3].end_col, {})[1]
  end
  return result, ranges
end

local function snapshot(buf)
  local state = M.states[buf]
  if not state or not api.nvim_buf_is_valid(buf) then return end
  local values = names(buf)
  if values and values[1] == values[2] and buf == api.nvim_get_current_buf() then
    state.name = values[1]
    state.tick = api.nvim_buf_get_changedtick(buf)
    state.sequence = vim.fn.undotree().seq_last
  end
end

local function companion(buf, changes)
  local state = M.states[buf]
  if not M.enabled or not state or not state.tick or #changes ~= 1 or buf ~= api.nvim_get_current_buf() then return false end
  local values, ranges = names(buf)
  if not values or values[1] == values[2] or state.tick + 1 ~= api.nvim_buf_get_changedtick(buf)
    or vim.fn.undotree().seq_cur ~= state.sequence + 1 then return false end
  local target = values[1] == state.name and 1 or values[2] == state.name and 2 or nil
  if not target then return false end
  local range, change = ranges[target], changes[1]
  if change[1] ~= range.row or change[3] ~= range.row or change[2] < range.first or change[4] > range.last
    or #change[5] ~= 1 then return false end
  local replacement = values[target]:sub(1, change[2] - range.first) .. change[5][1]
    .. values[target]:sub(change[4] - range.first + 1)
  return replacement == values[3 - target] and not replacement:find('[%s{}\\%%]')
end

local function wrapped(buf, changes)
  -- An incompatible future sync payload must still reach Neovim unchanged.
  local ok, join = pcall(companion, buf, changes)
  if ok and join then pcall(vim.cmd, 'undojoin') end
  local tick = original(buf, changes)
  if not pcall(snapshot, buf) then M.states[buf] = nil end
  return tick
end

function M.arm(data)
  if not M.enabled or type(original) ~= 'function' then return end
  for _, buf in ipairs(api.nvim_list_bufs()) do
    if api.nvim_buf_is_loaded(buf) and api.nvim_buf_get_name(buf) == data.path then
      -- A provider response may overtake a queued Neovim write. Arm only when
      -- both buffer names already match the version returned by the server.
      for _, range in ipairs(data.ranges) do
        local ok, text = pcall(api.nvim_buf_get_text, buf, range.row, range.first, range.row, range.last, {})
        if not ok or text[1] ~= data.name then return end
      end
      api.nvim_buf_clear_namespace(buf, namespace, 0, -1)
      for id, range in ipairs(data.ranges) do
        api.nvim_buf_set_extmark(buf, namespace, range.row, range.first, {
          id = id, end_row = range.row, end_col = range.last, right_gravity = false, end_right_gravity = true,
        })
      end
      M.states[buf] = {}
      api.nvim_buf_call(buf, function() snapshot(buf) end)
      if not M.attached[buf] then
        M.attached[buf] = api.nvim_buf_attach(buf, false, {
          on_lines = function()
            if not M.enabled then M.attached[buf] = nil; return true end
            if not pcall(snapshot, buf) then M.states[buf] = nil end
          end,
          on_detach = function() M.states[buf] = nil; M.attached[buf] = nil end,
        })
      end
      return
    end
  end
end

function M.clear(path)
  for buf in pairs(M.states) do
    if api.nvim_buf_is_valid(buf) and (not path or api.nvim_buf_get_name(buf) == path) then
      api.nvim_buf_clear_namespace(buf, namespace, 0, -1)
      M.states[buf] = nil
    end
  end
end

function M.dispose()
  M.clear()
  M.enabled = false
  if internal.handle_changes == wrapped then internal.handle_changes = original end
end

if type(original) == 'function' then internal.handle_changes = wrapped end
return M
