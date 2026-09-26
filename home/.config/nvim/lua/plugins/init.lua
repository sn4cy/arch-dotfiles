return {
  {
    "stevearc/conform.nvim",
    -- event = 'BufWritePre', -- uncomment for format on save
    opts = require "configs.conform",
  },

  -- These are some examples, uncomment them if you want to see them work!
  {
    "neovim/nvim-lspconfig",
    config = function()
      require "configs.lspconfig"
    end,
  },

  {
    "hrsh7th/nvim-cmp",
    opts = function(_, opts)
      -- Prefer language-aware LSP results over words from the current buffer.
      for _, source in ipairs(opts.sources or {}) do
        if source.name == "nvim_lsp" then
          source.priority = 1000
        elseif source.name == "luasnip" then
          source.priority = 750
        elseif source.name == "buffer" then
          source.priority = 250
          source.keyword_length = 3
        end
      end

      opts.completion = vim.tbl_deep_extend("force", opts.completion or {}, {
        completeopt = "menu,menuone,noinsert",
      })
      opts.experimental = vim.tbl_deep_extend("force", opts.experimental or {}, {
        ghost_text = true,
      })

      return opts
    end,
  },

  {
    "nvim-treesitter/nvim-treesitter",
    opts = {
      ensure_installed = {
        "bash",
        "c",
        "cpp",
        "css",
        "html",
        "javascript",
        "json",
        "lua",
        "luadoc",
        "markdown",
        "markdown_inline",
        "printf",
        "python",
        "regex",
        "rust",
        "toml",
        "tsx",
        "typescript",
        "vim",
        "vimdoc",
        "yaml",
      },
    },
  },
}
