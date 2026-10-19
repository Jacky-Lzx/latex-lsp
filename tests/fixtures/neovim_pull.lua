local ok, err = xpcall(function()
  local documented
  do
    local config, enable = vim.lsp.config, vim.lsp.enable
    vim.lsp.config = function(name, options)
      assert(name == "latex_lsp")
      documented = options
    end
    vim.lsp.enable = function() end
    dofile(assert(vim.env.LATEX_LSP_AUDIT_CONFIG))
    vim.lsp.config, vim.lsp.enable = config, enable
  end
  assert(documented and documented.handlers)
  local project = vim.fn.tempname()
  vim.fn.mkdir(project, "p")
  local source = {
    [[\documentclass{article}]],
    [[\begin{document}]],
    [[$\epsilonn$]],
    [[\input{chapter}]],
    [[\end{document}]],
  }
  vim.fn.writefile(source, project .. "/main.tex")
  vim.fn.writefile({ [[\PackageWarning{audit}{Hidden child warning}]] }, project .. "/chapter.tex")
  vim.cmd.edit(vim.fn.fnameescape(project .. "/main.tex"))
  local buffer, finished, pushes, refreshes = vim.api.nvim_get_current_buf(), nil, 0, 0
  local native_refresh = documented.handlers["workspace/diagnostic/refresh"]
  local id = vim.lsp.start({
    name = "latex_lsp",
    cmd = { vim.env.LATEX_LSP_AUDIT_BIN or vim.fn.getcwd() .. "/target/release/latex-lsp", "--stdio" },
    root_dir = project,
    init_options = { build = { executable = "pdflatex" } },
    capabilities = documented.capabilities,
    handlers = {
      ["latex/buildStarted"] = function() end,
      ["latex/buildFinished"] = function(_, result)
        finished = result
      end,
      ["textDocument/publishDiagnostics"] = function()
        pushes = pushes + 1
      end,
      ["workspace/diagnostic/refresh"] = function(...)
        refreshes = refreshes + 1
        return native_refresh(...)
      end,
    },
  })
  local client = vim.lsp.get_client_by_id(id)
  assert(vim.wait(10000, function()
    return client.initialized
  end, 20))
  assert(client.server_capabilities.diagnosticProvider, "missing negotiated diagnostic provider")
  local uri = vim.uri_from_fname(project .. "/main.tex")
  local function request(method, params)
    local response = client:request_sync(method, params, 5000, buffer)
    assert(response and not response.err, vim.inspect(response))
    return response.result
  end
  local function compile(status)
    finished = nil
    request("workspace/executeCommand", { command = "latex.build", arguments = { uri } })
    assert(vim.wait(20000, function()
      return finished ~= nil
    end, 20))
    assert(finished.status == status and not finished.stale, vim.inspect(finished))
  end
  compile("error")
  assert(
    vim.wait(5000, function()
      return vim.iter(vim.diagnostic.get(buffer)):any(function(d)
        return d.code == "undefined-control-sequence"
      end)
    end, 20),
    "native refresh did not display compiler diagnostics: " .. vim.inspect(vim.diagnostic.get(buffer))
  )
  local diagnostic = vim.iter(vim.diagnostic.get(buffer)):find(function(d)
    return d.code == "undefined-control-sequence"
  end)
  assert(diagnostic.lnum == 2 and diagnostic.col == 1, vim.inspect(diagnostic))
  local child = vim.uri_to_bufnr(vim.uri_from_fname(project .. "/chapter.tex"))
  assert(
    vim.wait(5000, function()
      return vim.iter(vim.diagnostic.get(child)):any(function(d)
        return d.source == "latex-build" and d.message:find("Hidden child warning", 1, true)
      end)
    end, 20),
    "workspace refresh did not display the closed child diagnostic"
  )
  local first = request("textDocument/diagnostic", { textDocument = { uri = uri }, identifier = "latex-lsp" })
  local unchanged = request(
    "textDocument/diagnostic",
    { textDocument = { uri = uri }, identifier = "latex-lsp", previousResultId = first.resultId }
  )
  assert(unchanged.kind == "unchanged", vim.inspect(unchanged))
  local raw = vim.iter(first.items):find(function(d)
    return d.code == "undefined-control-sequence"
  end)
  local actions = request(
    "textDocument/codeAction",
    { textDocument = { uri = uri }, range = raw.range, context = { diagnostics = { raw }, only = { "quickfix" } } }
  )
  local action = vim.iter(actions):find(function(a)
    return a.title == [[Replace with command `\epsilon`]]
  end)
  assert(action and action.edit, vim.inspect(actions))
  vim.lsp.util.apply_workspace_edit(action.edit, "utf-16")
  vim.cmd.write()
  assert(
    vim.wait(5000, function()
      return #vim.diagnostic.get(buffer) == 0
    end, 20),
    vim.inspect(vim.diagnostic.get(buffer))
  )
  compile("success")
  local workspace = request("workspace/diagnostic", { identifier = "latex-lsp", previousResultIds = {} })
  local main = vim.iter(workspace.items):find(function(d)
    return d.uri == uri
  end)
  assert(main and main.kind == "full" and #main.items == 0, vim.inspect(workspace))
  local child_uri = vim.uri_from_fname(project .. "/chapter.tex")
  vim.fn.delete(project .. "/chapter.tex")
  client:notify("workspace/didDeleteFiles", { files = { { uri = child_uri } } })
  assert(
    vim.wait(5000, function()
      return #vim.diagnostic.get(child) == 0
    end, 20),
    "deleting the closed source did not clear its compiler diagnostic"
  )
  vim.fn.writefile({ [[\PackageWarning{audit}{Recreated child warning}]] }, project .. "/chapter.tex")
  client:notify("workspace/didCreateFiles", { files = { { uri = child_uri } } })
  compile("success")
  assert(
    vim.wait(5000, function()
      return vim.iter(vim.diagnostic.get(child)):any(function(d)
        return d.source == "latex-build" and d.message:find("Recreated child warning", 1, true)
      end)
    end, 20),
    "recreating the source did not restore compiler diagnostics"
  )
  assert(pushes == 0, "pull client received pushed diagnostics")
  assert(refreshes >= 2, "missing refresh after compilation")
  client:stop()
  assert(vim.wait(5000, function()
    return client:is_stopped()
  end, 20))
  vim.fn.delete(project, "rf")
  print(
    "Neovim pull diagnostic audit passed: native compilation refresh, precise errors, versioned typo fix, closed source deletion/recreation and successful recompilation."
  )
end, debug.traceback)
if not ok then
  io.stderr:write(err .. "\n")
  vim.cmd("cquit 1")
else
  vim.cmd("qa!")
end
