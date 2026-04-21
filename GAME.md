In this game you are against the clock to turn off all red buttons as they appear. If all of the buttons are on then it's game over. However, if you press a button that's not red it will turn red.

## Game Rules

### Button Behaviour
- Buttons stay red until pressed
- A button can immediately re-light after being cleared

### Penalties
- Pressing an unlit button turns it red — no other penalty

### Starting State
- 1 random button starts lit

### Win/Lose
- No win condition — survive as long as possible
- Game over when all 16 buttons are lit (no dark spaces left)
- On game over — all buttons go dim red and the game freezes
- Press any button to start a fresh run

### Spawn Rate
```
interval = max(0.15, 3.0 - (seconds_alive * 0.01) + (lit_buttons * 0.1))
```
- Starts spawning a new button every ~3 seconds
- Gets faster the longer you survive (plateau reached around 285s / ~4m45s alive)
- Slows down when many buttons are lit (lets you recover)
- Never faster than 0.15 seconds

### Special Buttons
Each time a button spawns it has a chance of being a special button instead of red:
- 90% chance — normal red button
- 5% chance — gold button
- 5% chance — purple button (only one on the board at a time)

**Once the spawn rate plateaus (~285s alive), special rates double:**
- 80% red / 10% gold / 10% purple
- Up to 2 purples can be on the board at the same time

**Gold button:**
- Press it → clears itself plus up to 2 random red buttons from anywhere on the board
- If fewer than 2 red buttons exist, clears what's there
- If ignored for 3 seconds → turns red like a normal button

**Purple button:**
- No grace period — starts ticking immediately on spawn
- Adds one random red button every 1 second until pressed
- Press it → purple turns off, but any red buttons it caused stay lit

**Blue Boss button:**
- First spawns after ~115 seconds alive (fixed timer, independent of spawn ramp)
- Spawns on a dark space once every 30 seconds
- Only one on the board at a time
- On spawn → instantly fills its row and column with red minions (only dark spaces, existing lit buttons are untouched)
- Minions cannot be cleared while the boss is still blue
- Press it → disappears safely, minions become normal clearable red buttons
- If ignored for 3 seconds → turns red, minions become clearable, clears like a normal red button

