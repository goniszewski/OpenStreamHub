-- Spotify for the Now Playing plugin. Arguments arrive in argv, never inside the script.
-- The plugin runs this only once Spotify is running, so a Mac without Spotify never compiles it.
-- state: "stopped", or the state, track id, name, artist and artwork URL of the current track, tab-separated.
-- playpause, next, previous: the transport.
on run argv
	set command to item 1 of argv
	if application id "com.spotify.client" is not running then
		if command is not "playpause" then return "not_running"
	end if
	tell application id "com.spotify.client"
		if command is "playpause" then
			playpause
			return "ok"
		else if command is "next" then
			next track
			return "ok"
		else if command is "previous" then
			previous track
			return "ok"
		else if command is not "state" then
			error "unknown command " & command
		end if
		set playerState to player state as text
		if playerState is "stopped" then return "stopped"
		try
			set theTrack to current track
			return playerState & tab & (id of theTrack) & tab & (name of theTrack) & tab & (artist of theTrack) & tab & (artwork url of theTrack)
		on error
			return "stopped"
		end try
	end tell
end run
