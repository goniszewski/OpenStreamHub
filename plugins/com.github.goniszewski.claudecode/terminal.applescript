-- Finds a Terminal tab by its tty and looks at it, brings it to the front, or answers its prompt.
-- Run as: osascript terminal.applescript <verb> <ttys> [<count> <text>]
-- <ttys> are the terminals of the Claude Code process and those above it, separated by commas; the tab is the one
-- whose tty is one of them.
-- - look: prints "true" or "false" (whether the tab is the selected tab of the front window, with Terminal the
--   frontmost app), a tab, the tab's tty, a line break, then the tab's text.
-- - focus: selects the tab, brings its window and Terminal to the front.
-- - return, escape: presses the key (through System Events, so OpenStreamHub needs Accessibility), only while the tab
--   is in front, Terminal is the frontmost app, and its last <count> lines (the prompt) are still exactly <text>,
--   what the plugin read and checked; else prints "not_front" or "changed".
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
	set AppleScript's text item delimiters to ","
	set ttys to text items of (item 2 of argv)
	set AppleScript's text item delimiters to ""
	if application "Terminal" is not running then return "missing"
	set found to false
	tell application "Terminal"
		repeat with w in windows
			repeat with t in tabs of w
				if ttys contains (tty of t as text) then
					if verb is "focus" then
						set selected of t to true
						set index of w to 1
						activate
						return "ok"
					end if
					set isFront to frontmost and (index of w is 1) and (selected of t)
					if verb is "look" then return (isFront as text) & (character id 9) & (tty of t as text) & linefeed & (contents of t)
					if not isFront then return "not_front"
					considering case
						set expected to item 4 of argv
						if my lastLines(contents of t, (item 3 of argv) as integer) is not expected then return "changed"
					end considering
					set found to true
					exit repeat
				end if
			end repeat
			if found then exit repeat
		end repeat
	end tell
	if not found then return "missing"
	tell application "System Events"
		if name of first application process whose frontmost is true is not "Terminal" then return "not_front"
		if verb is "return" then
			key code 36
		else if verb is "escape" then
			key code 53
		else
			return "unknown verb"
		end if
	end tell
	return "ok"
end run
