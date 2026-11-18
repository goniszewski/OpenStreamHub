-- Apple Music for the Now Playing plugin. Arguments arrive in argv, never inside the script.
-- state: "stopped", or the state, persistent ID, name and artist of the current track, tab-separated.
-- playpause, next, previous: the transport; only playpause starts Music when it is not running.
-- artwork <path>: writes the current track's first artwork to <path>, answers "ok" or "none".
on run argv
	set command to item 1 of argv
	if application id "com.apple.Music" is not running then
		if command is not "playpause" then return "not_running"
	end if
	tell application id "com.apple.Music"
		if command is "playpause" then
			playpause
			return "ok"
		else if command is "next" then
			next track
			return "ok"
		else if command is "previous" then
			previous track
			return "ok"
		end if
		set playerState to player state as text
		try
			set theTrack to current track
		on error
			return "stopped"
		end try
		if command is "state" then
			if playerState is "stopped" then return "stopped"
			return playerState & tab & (persistent ID of theTrack) & tab & (name of theTrack) & tab & (artist of theTrack)
		end if
		if command is not "artwork" then error "unknown command " & command
		if (count of artworks of theTrack) is 0 then return "none"
		set coverData to raw data of artwork 1 of theTrack
	end tell
	set target to open for access (POSIX file (item 2 of argv)) with write permission
	try
		set eof target to 0
		write coverData to target
		close access target
	on error message
		close access target
		error message
	end try
	return "ok"
end run
