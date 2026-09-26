require "nvchad.autocmds"

local autosave_group = vim.api.nvim_create_augroup("autosave_on_insert_leave", { clear = true })

vim.api.nvim_create_autocmd("InsertLeave", {
  group = autosave_group,
  desc = "Save modified files after leaving Insert mode",
  callback = function(args)
    local bufnr = args.buf

    if vim.bo[bufnr].buftype ~= ""
      or not vim.bo[bufnr].modifiable
      or vim.bo[bufnr].readonly
      or not vim.bo[bufnr].modified
      or vim.api.nvim_buf_get_name(bufnr) == ""
    then
      return
    end

    vim.api.nvim_buf_call(bufnr, function()
      vim.cmd "silent update"
    end)
  end,
})
