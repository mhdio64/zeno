# Saving is atomic and preserves the file's format

Saving writes a temporary file in the target's directory and renames it over the original, and the original's permission bits, encoding, BOM, line endings and final-newline presence are preserved. Rewriting the file in place would have preserved its inode and any hard links, but a crash mid-write would leave a truncated file, whereas a crash mid-rename leaves the original untouched. The cost is that saving breaks hard links to the edited file, and that a file whose directory is not writable cannot be saved even when the file itself is.
