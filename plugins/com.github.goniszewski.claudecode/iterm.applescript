-- Finds an iTerm2 session by its unique id and looks at it, brings it to the front, or answers its prompt.
-- Run as: osascript iterm.applescript <verb> <session unique id> <ttys> [<count> <text>]
-- <ttys> are the terminals of the Claude Code process and those above it, separated by commas: a session whose tty
-- is not one of them (another tmux pane, an app started from that session) is treated as not found ("missing").
-- - look: prints "true" or "false" (whether the session is the one in front, with iTerm2 the frontmost app), a tab,
--   the session's tty, a line break, then the session's text.
-- - focus: selects its window, tab and session and brings iTerm2 to the front.
-- - return, escape: types the key into the session, only while it is in front and its last <count> lines (the
--   prompt) are still exactly <text>, what the plugin read and checked; else prints "not_front" or "changed".
on trimEnd(value)
	set n to length of value
	repeat while n > 0
		set c to character n of value
		if c is not linefeed and c is not return then exit repeat
		set n to n - 1
	end repeat
	if n = 0 then return ""
	return text 1 thru n of value
end trimEnd

on lastLines(value, wanted)
	set pieces to paragraphs of my trimEnd(value)
	if (count pieces) < wanted or wanted < 1 then return missing value
	set AppleScript's text item delimiters to linefeed
	set joined to (items ((count pieces) - wanted + 1) thru -1 of pieces) as text
	set AppleScript's text item delimiters to ""
	return joined
end lastLines

on run argv
	set verb to item 1 of argv
	set wanted to item 2 of argv
	set AppleScript's text item delimiters to ","
	set ttys to text items of (item 3 of argv)
	set AppleScript's text item delimiters to ""
	if application "iTerm2" is not running then return "missing"
	tell application "iTerm2"
		repeat with w in windows
			repeat with t in tabs of w
				repeat with s in sessions of t
					if unique id of s is wanted then
						if ttys does not contain (tty of s as text) then return "missing"
						if verb is "focus" then
							select w
							tell t to select
							tell s to select
							activate
							return "ok"
						end if
						set isFront to false
						if frontmost then
							try
								set isFront to (unique id of current session of current tab of current window) is wanted
							end try
						end if
						if verb is "look" then return (isFront as text) & (character id 9) & (tty of s as text) & linefeed & (contents of s)
						if not isFront then return "not_front"
						considering case
							set expected to item 5 of argv
							if my lastLines(contents of s, (item 4 of argv) as integer) is not expected then return "changed"
						end considering
						if verb is "return" then
							tell s to write text (character id 13) newline no
						else if verb is "escape" then
							tell s to write text (character id 27) newline no
						else
							return "unknown verb"
						end if
						return "ok"
					end if
				end repeat
			end repeat
		end repeat
	end tell
	return "missing"
end run
