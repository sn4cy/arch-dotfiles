require("nvchad.configs.lspconfig").defaults()

-- NvChad disables LSP semantic tokens by default.  Keep them enabled so that
-- servers can colour symbols by meaning (function, method, type, parameter,
-- etc.) in the same way as modern IDEs.
vim.lsp.config("*", {
  on_init = function() end,
})

vim.lsp.config("pyright", {
  -- Also provide IDE features for a standalone .py file.  Without a project
  -- marker, nvim-lspconfig intentionally leaves the server detached.
  root_dir = function(bufnr, on_dir)
    local markers = {
      "pyrightconfig.json",
      "pyproject.toml",
      "setup.py",
      "setup.cfg",
      "requirements.txt",
      "Pipfile",
      ".git",
    }
    local filename = vim.api.nvim_buf_get_name(bufnr)
    on_dir(vim.fs.root(bufnr, markers) or vim.fs.dirname(filename) or vim.fn.getcwd())
  end,
  settings = {
    python = {
      analysis = {
        autoImportCompletions = true,
        autoSearchPaths = true,
        diagnosticMode = "workspace",
        typeCheckingMode = "basic",
        useLibraryCodeForTypes = true,
      },
    },
  },
})

local servers = { "pyright", "html", "cssls" }
vim.lsp.enable(servers)
