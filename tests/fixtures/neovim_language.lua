local ok, err = xpcall(function()
  vim.cmd('edit newrefs.data')
  vim.bo.filetype = 'bib'
  local buffer = vim.api.nvim_get_current_buf()
  vim.api.nvim_buf_set_lines(buffer, 0, -1, false, { '@book{first,title={First}}' })
  local id = assert(vim.lsp.start({
    name = 'texquill',
    cmd = { assert(vim.env.TEXQUILL_TEST_BIN), '--stdio' },
    root_dir = vim.fn.getcwd(),
  }))
  local client = assert(vim.lsp.get_client_by_id(id))
  assert(vim.wait(10000, function() return client.initialized end, 20))
  local uri = vim.uri_from_bufnr(buffer)
  local function request(method, params)
    local response = client:request_sync(method, params, 5000, buffer)
    assert(response and not response.err, vim.inspect(response))
    return response.result
  end
  local function symbols()
    return request('textDocument/documentSymbol', { textDocument = { uri = uri } })
  end
  assert(symbols()[1].name == 'first')
  vim.api.nvim_buf_set_text(buffer, 0, 6, 0, 11, { 'second' })
  assert(symbols()[1].name == 'second')
  vim.api.nvim_buf_set_lines(buffer, 0, -1, false, { '@art' })
  local items = request('textDocument/completion', {
    textDocument = { uri = uri }, position = { line = 0, character = 4 },
  })
  local article
  for _, item in ipairs(items) do
    if item.label == 'article' then article = item end
  end
  assert(article, vim.inspect(items))
  assert(article.textEdit.range.start.character == 1)
  assert(vim.fn.filereadable('newrefs.data') == 0)
  client:stop(true)
end, debug.traceback)
if not ok then
  io.stderr:write(err .. '\n')
  vim.cmd('cquit 1')
end
vim.cmd('qa!')
