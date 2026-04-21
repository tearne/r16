The game has two modes: **Endless** and **Story**.

In both modes you are against the clock to turn off all red buttons as they appear. If all of the buttons are on then it's game over. However, if you press a button that's not red it will turn red.

## Game Rules

### Button Behaviour
- Buttons stay red until pressed
- A button can immediately re-light after being cleared

### Penalties
- Pressing an unlit button turns it red — no other penalty

### Starting State
- 1 random button starts lit

### Win/Lose

**Endless mode:**
- No win condition — survive as long as possible
- Game over when all 16 buttons are lit (no dark spaces left)
- On game over — all buttons go dim red and the game freezes
- Press any button to start a fresh run

**Story mode:**
- Same spawn rate and special button mechanics as endless mode, plus additional mechanics
- Survive 5 boss fights — board carries over between fights
- Each fight the boss stays locked longer and the spawn rate starts faster
- After the 5th boss is cleared, a 10-second break begins — no buttons spawn, giving you time to clear up
- Then the Final Boss spawns
- Defeat the Final Boss to win the game
- Game over when all 16 buttons are lit — same as endless

| Fight | Boss timeout | Spawn rate head start |
|-------|-------------|----------------------|
| 1     | 5 seconds   | 0s (normal start)    |
| 2     | 7 seconds   | 30s head start       |
| 3     | 9 seconds   | 60s head start       |
| 4     | 11 seconds  | 90s head start       |
| 5     | 13 seconds  | 120s head start      |

### Spawn Rate
```
interval = max(0.05, 3.0 - (seconds_alive * 0.01) + (lit_buttons * 0.1))
```
- Starts spawning a new button every ~3 seconds
- Gets faster the longer you survive (plateau reached around 285s / ~4m45s alive)
- Slows down when many buttons are lit (lets you recover)
- Never faster than 0.05 seconds

### Buttons

Each time a button spawns it has a chance of being a special button instead of red:
- 85% chance — normal red button
- 5% chance — gold button
- 10% chance — purple button (up to 2 on the board at a time)

---

**Both modes:**

**Gold button:**
- Press it → clears itself plus up to 2 random red buttons from anywhere on the board
- If fewer than 2 red buttons exist, clears what's there
- If ignored for 3 seconds → turns red like a normal button

**Purple button:**
- No grace period — starts ticking immediately on spawn
- Spawns one random button (any type — red, gold, or purple) every 1 second until pressed
- Press it → purple turns off, but any buttons it caused stay lit

**Phantom button:**
- 10% chance to spawn instead of a red button
- Flickers on and off on a fixed 1 second on / 1 second off pattern
- Only pressable when lit
- Counts as a lit button toward game over even when dark
- Press it when lit to clear it normally

**Anchor button:**
- 5% chance to spawn, appears green
- Cannot be cleared while it has 4 or more lit neighbours
- Clear neighbouring buttons first to free it, then press to clear normally

**Boss button:**
- First spawns after ~115 seconds alive (fixed timer, independent of spawn ramp)
- Spawns on a dark space once every 30 seconds in endless mode, every 60 seconds in story mode
- Only one on the board at a time
- On spawn → instantly fills its row and column with red minions (only dark spaces, existing lit buttons are untouched)
- Minions cannot be cleared while the boss is still blue
- Press it → disappears safely, minions become normal clearable red buttons
- If ignored for 3 seconds → turns red, minions become clearable, clears like a normal red button

---

**Story mode only:**

**Quiet button:**
- 5% chance to spawn, appears as a red button at half brightness — easy to overlook
- Behaves like a normal red button in all other ways

**Shields:**
- Any red button has a 25% chance of being shielded
- Shielded buttons look identical to normal red buttons
- First press breaks the shield — the button still appears red
- Second press clears it normally
- A button can be both shielded and a decoy — first press breaks the shield, second press triggers the decoy effect

**Decoy button:**
- Any red button has a 10% chance of being a decoy
- A decoy looks identical to a red button but is actually a purple button in disguise
- Behaves exactly like a purple button — spawns one random button every second until pressed
- Press it → stops spawning and disappears

**Final Boss:**
- Appears as a rainbow button — distinct from the regular blue boss
- Spawns after surviving 5 boss fights
- Shields and decoys are inactive for the duration of this fight
- Normal spawn rate is replaced entirely by 1 red button per second for the whole fight
- The boss and 3 white bodyguard buttons appear on the board — the boss is hidden among them
- All 3 bodyguards must be pressed to reveal which button is the real boss
- Bodyguards stay white until pressed and teleport to a random dark space every few seconds
- Reds keep spawning even after the boss is revealed
- Press the boss → it survives, 3 new bodyguards spawn, and the round escalates
- 3 rounds total — after the 3rd hit the Final Boss is defeated and the game is won

| Round | Burst of reds on start | Bodyguard teleport interval |
|-------|------------------------|----------------------------|
| 1     | 3                      | every 2 seconds             |
| 2     | 6                      | every 1.5 seconds           |
| 3     | 9                      | every 1 second              |
