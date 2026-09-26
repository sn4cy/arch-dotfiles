require "nvchad.mappings"

-- add yours here

local map = vim.keymap.set

map("n", ";", ":", { desc = "CMD enter command mode" })
map("i", "jk", "<ESC>")
map("n", "<leader>-", "<cmd>split<CR><C-w>w", { desc = "Split window horizontally" })
map("n", "<leader>|", "<cmd>vsplit<CR><C-w>w", { desc = "Split window vertically" })
map("n", "<leader>wd", "<cmd>close<CR>", { desc = "Close focused window" })

-- map({ "n", "i", "v" }, "<C-s>", "<cmd> w <cr>")
