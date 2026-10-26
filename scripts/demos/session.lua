-- Presentation and evidence only. All language features come from the real LSP
-- client and the user's installed Neovim configuration.
local scene = assert(vim.env.TEXQUILL_DEMO_SCENE)
local evidence = assert(vim.env.TEXQUILL_DEMO_EVIDENCE)
local report = { scene = scene, checkpoints = {} }
local notify = vim.notify
report.messages = {}
vim.notify = function(message, level, options)
  report.messages[#report.messages + 1] = tostring(message)
  return notify(message, level, options)
end
local function snapshot(name)
  local buffers = {}
  for _, buf in ipairs(vim.api.nvim_list_bufs()) do
    if vim.bo[buf].buflisted then
      buffers[vim.fn.fnamemodify(vim.api.nvim_buf_get_name(buf), ':~:.')] =
        vim.api.nvim_buf_get_lines(buf, 0, -1, false)
    end
  end
  local floats = {}
  for _, win in ipairs(vim.api.nvim_list_wins()) do
    if vim.api.nvim_win_get_config(win).relative ~= '' then
      floats[#floats + 1] = vim.api.nvim_buf_get_lines(vim.api.nvim_win_get_buf(win), 0, -1, false)
    end
  end
  report.checkpoints[#report.checkpoints + 1] = {
    name = name, buffers = buffers, floats = floats,
    diagnostics = vim.diagnostic.get(), cursor = vim.api.nvim_win_get_cursor(0),
    current = vim.fn.expand('%:.'), mode = vim.fn.mode(),
    completion = package.loaded['blink.cmp'] and (function()
      local item = require('blink.cmp').get_selected_item()
      return item and { label = item.label, source = item.source_name, filterText = item.filterText } or nil
    end)() or nil,
  }
  vim.fn.writefile({ vim.json.encode(report) }, evidence)
end
local function title(text)
  vim.wo.winbar = '%#Title#  TeXquill  %* /  ' .. text
end
local function find(text, offset)
  for row, line in ipairs(vim.api.nvim_buf_get_lines(0, 0, -1, false)) do
    local col = line:find(text, 1, true)
    if col then
      vim.api.nvim_win_set_cursor(0, { row, col - 1 + (offset or 0) })
      return
    end
  end
  error('Demo text not found: ' .. text)
end

vim.defer_fn(function()
  local ok, err = pcall(function()
    assert(vim.wait(12000, function() return #vim.lsp.get_clients({ name = 'texquill', bufnr = 0 }) == 1 end), 'TeXquill did not attach')
    local client = vim.lsp.get_clients({ name = 'texquill', bufnr = 0 })[1]
    report.client = { name = client.name, cmd = client.config.cmd, config = vim.fn.stdpath('config'), version = vim.version() }
    report.build_maps = {}
    for _, key in ipairs({ ' lb', ' ll' }) do
      local map = vim.fn.maparg(key, 'n', false, true)
      report.build_maps[key] = { desc = map.desc, rhs = map.rhs }
    end
    -- Keep source text readable in the recording; theme, completion, popups,
    -- mappings, linked editing, and statusline remain the user's configuration.
    vim.wo.conceallevel = 0
    vim.wo.relativenumber = false
    vim.opt.swapfile = false
    if vim.fn.exists(':Copilot') == 2 then
      vim.cmd('Copilot disable')
    end
    if scene == 'completion' then
      title('Find a citation by its title')
      find('cite{', #'cite{')
    elseif scene == 'navigation' then
      title('Follow a reference across files  /  gd')
      find('sec:method', 5)
    elseif scene == 'diagnostics' then
      title('Fix a misspelled reference  /  Space a')
      find('sec:metohd', 5)
    elseif scene == 'linked-editing' then
      title('Edit one environment name; update both')
      find('begin{quote}', #'begin{')
    elseif scene == 'signatures' then
      title('Documentation from your own macro definitions  /  K')
      find('term{', 1)
      vim.keymap.set('n', '<C-j>', function()
        title('Cross-file signatures  /  gK')
        find('28.4', 2)
      end)
    elseif scene == 'build' then
      title('Format LaTeX with the language server')
      vim.keymap.set('n', '<C-j>', function() vim.lsp.buf.format({ name = 'texquill', async = true }) end)
      vim.keymap.set('n', '<C-l>', function() title('Build the paper  /  Space l b') end)
    end
    if scene == 'navigation' then
      vim.keymap.set('n', '<C-j>', function()
        vim.cmd('vsplit main.tex')
        find('sec:method', 5)
        vim.cmd('wincmd h')
        find('sec:method', 5)
        title('Rename a label throughout the project  /  Space r n')
      end)
    end
    snapshot('ready')
    vim.cmd('redraw!')
    local function sample()
      pcall(snapshot, 'sample')
      vim.defer_fn(sample, 700)
    end
    vim.defer_fn(sample, 700)
  end)
  if not ok then
    report.error = tostring(err)
    vim.fn.writefile({ vim.json.encode(report) }, evidence)
  end
end, 1500)
