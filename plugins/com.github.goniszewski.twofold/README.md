# Twofold

An OpenStreamHub plugin: a sliding number game on the M4315's 15 keys, played with the M032LE rollers. Slide every tile toward one edge; two equal tiles that meet merge into one of twice the value, and the merge scores that value. Reach 2048, then keep going for as long as the board lets you.

| | |
| --- | --- |
| **Board** | 3 rows of 5: each key is the cell where it sits. |
| **Moves** | One detent of a roller is one move: the first roller up and down moves the tiles up and down, the second left and right (any key, knob or dial turn can make a move too). |
| **Score** | Hold any key for a second: the middle row shows the score and the best score while you hold it. |
| **New game** | Press any key twice quickly: the middle row asks **NEW GAME?**; press **YES** (the middle key) to start again. Anything else, or a move, goes on with the game, and the question goes away after 5 seconds. A press in the first moment after the question comes up does not answer it. |
| **Game over** | When no move is left the middle row says **GAME OVER** with the score; a press starts a new game. |

The rules are the classic sliding-merge ones: a move counts only when it changes the board; a tile made by a merge does not merge again in the same move; after each move a 2 (one time in ten a 4) appears in a random empty cell. Reaching 2048 is shown for a moment, once a game. The game is kept, so it waits for you across restarts of the app or the computer.

Requires OpenStreamHub 0.5.0 or later, macOS, and `/usr/bin/python3` (it comes with the Xcode command-line tools). It needs no permissions: it uses no network, starts no programs, and draws its keys itself. Status: unit-tested; played through a fake app socket on 17 November 2026. Not yet played on a real M4315 and M032LE.

## Set up

1. In OpenStreamHub, open **Settings > Plugins**, choose **Install…**, pick this folder (`plugins/com.github.goniszewski.twofold` in a copy of the repository) and choose **Install and enable**.
2. Make a keypad page for the game and bind each of its 15 keys' **Press** to the **Plugin** action (Apps & web), with **Twofold** as the plugin and **Board key** as its action. Leave their Release unbound: a hold is told apart only on a key the app draws for this action.
3. Bind the rollers to **Twofold**'s **Move** action, one binding per direction: the first roller's **Up** to **Up** and **Down** to **Down**, the second roller's **Up** to **Left** and **Down** to **Right**.
4. Run the profile in **Live**, with Settings > Let OpenStreamHub drive the display set to **App**.

The board follows the keys' wire order (top-left is the first). On a keypad turned on its base the board turns with it, but the rollers do not: pick the directions that match how you look at it.

## What it does on your Mac

It keeps the game (board, score, best score) in `game.json` in its data folder (`~/Library/Application Support/dev.openstreamhub/plugin-data/com.github.goniszewski.twofold`). Keys are drawn by `picture.py`, a small PNG writer with its own pixel font, and only keys whose picture changed are sent, at most 15 a second, well under the app's limit of 20 (what that holds back goes out a moment later).

## Tests

From this folder: `/usr/bin/python3 -B -m unittest discover -s tests`.
