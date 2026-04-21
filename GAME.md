In this game you will be agianst the clock to turn off all red buttons as they appear, if all of the buttons on are then its game over. However if you press a button thats not red it will turn red.

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
- Game over when all 16 buttons are red
- On game over — all buttons stay red, game just stops

### Spawn Rate
```
interval = max(0.5, 3.0 - (seconds_alive * 0.05) + (lit_buttons * 0.1))
```
- Starts spawning a new button every ~3 seconds
- Gets faster the longer you survive
- Slows down when many buttons are lit (lets you recover)
- Never faster than 0.5 seconds

### Special Buttons
Each time a button spawns it has a chance of being a special button instead of red:
- 90% chance — normal red button
- 5% chance — gold button
- 5% chance — purple button (only one on the board at a time)

**Gold button:**
- Press it → clears itself plus up to 2 random red buttons from anywhere on the board
- If fewer than 2 red buttons exist, clears what's there
- If ignored for 3 seconds → turns red like a normal button

**Purple button:**
- 2 second grace period after spawning before it starts causing damage
- After grace period → adds one random red button every 5 seconds until pressed
- Press it → purple turns off, but any red buttons it caused stay lit

**Blue Boss button:**
- Only appears once max spawn rate is reached
- Spawns on a dark space once every minute
- Only one on the board at a time
- On spawn → instantly fills its row and column with red minions
- Minions cannot be cleared while the boss is still blue
- Press it → disappears safely, minions become normal clearable red buttons
- If ignored → turns red, minions become clearable, clears like a normal red button

