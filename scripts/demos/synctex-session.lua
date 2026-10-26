-- Real Neovim/Skim session; the harness only records responses and navigation.
local output = assert(vim.env.TEXQUILL_SYNCTEX_EVIDENCE)
local report = { requests = {}, inverse = {}, samples = {} }
local function save()
  vim.fn.writefile({ vim.json.encode(report) }, output)
end
vim.api.nvim_create_autocmd('User', {
  pattern = 'TexToolsInverseSearch',
  callback = function(event)
    report.inverse[#report.inverse + 1] = event.data
    save()
  end,
})
vim.defer_fn(function()
  local ok, err = pcall(function()
    assert(vim.wait(15000, function()
      return #vim.lsp.get_clients({ name = 'texquill', bufnr = 0 }) == 1
    end), 'TeXquill did not attach')
    local client = vim.lsp.get_clients({ name = 'texquill', bufnr = 0 })[1]
    report.config = vim.fn.stdpath('config')
    report.server = client.config.cmd
    local original = client.request
    client.request = function(self, method, params, handler, bufnr)
      if method == 'workspace/executeCommand' then
        local callback = handler
        handler = function(error, result, context, config)
          report.requests[#report.requests + 1] = { params = params, error = error, result = result }
          save()
          if callback then return callback(error, result, context, config) end
        end
      end
      return original(self, method, params, handler, bufnr)
    end
    if vim.fn.exists(':Copilot') == 2 then vim.cmd('Copilot disable') end
    for _, name in ipairs({ 'main.tex', 'sections/method.tex', 'sections/results.tex' }) do
      local buf = vim.fn.bufadd(name)
      vim.fn.bufload(buf)
    end
    vim.cmd('edit sections/results.tex')
    vim.wo.conceallevel = 0
    vim.wo.relativenumber = false
    vim.api.nvim_win_set_cursor(0, { 4, 0 })
    require('tex-tools.inverse').register()
    client:exec_cmd({ command = 'latex.build', arguments = { vim.uri_from_fname(vim.fn.getcwd() .. '/main.tex') } }, { bufnr = 0 })
    local function sample()
      report.samples[#report.samples + 1] = { file = vim.fn.expand('%:.'), cursor = vim.api.nvim_win_get_cursor(0) }
      save()
      vim.defer_fn(sample, 300)
    end
    report.ready = true
    sample()
  end)
  if not ok then report.error = tostring(err); save() end
end, 1500)
