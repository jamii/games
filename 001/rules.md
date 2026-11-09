## basics

Hex board. Looks like:

   x H H x
  x x H x x
 x x x x x x
x x x F x x x
 x x x x x x
  x x H x x  
   x H H x

H: home
F: flag
x: empty

Setup:
* Find the *Walk* power and put the *Free Power X* token on it.
* Deal 6 power cards faceup beside the board.
* Take turns drawing until each player has 3 cards. You can draw either:
  * A faceup card. Slide the faceup cards to its right to the left to fill the gap, then draw a new faceup card on the right.
  * The top card of the deck.
* Put the flag in the space marked 'F', with the arrow pointing to one side of the board. The flag is a neutral unit with the *Flag* power.
* First player starts with 0 points, second player with 1 point.

Advanced options:
* Replace *Walk* with a different movement power.
* Draw a power from the deck and put the *Free Power Y* token on it.
* Draw a power from the deck and add it to the flag.
  * If it's an active power, either player can activate it on their turn.

Each turn:
1. Discard the leftmost faceup card from the table. Slide all faceup cards to the left. Draw a new faceup card on the right.
2. Draw 2 cards.
3. In any order, all optional:
  * Spawn a unit in one of your home spaces.
    * Only once per turn.
  * Apply a power from your hand to a unit in one of your home spaces.
  * Apply a free power to a unit in one of your home spaces.
  * Discard a power from a unit in your home space.
  * Activate one of your units:
    * Rotate the unit to face in any direction.
    * Optionally, use one of the active powers on the unit.
    * Each unit can only be activated once per turn.

At the end of your turn, if you have more units adjacent to the *Flag* power then the enemy, score 1 point.

The first player to score 5 points wins.

## units and powers

You can have up to five units. If you ever create a sixth unit, you must immediately kill one of your units.

Each unit can have up to three powers.

If a fourth power would be added for any reason, the player whose turn it is decides which of the existing powers to replace.

A unit can have multiple copies of the same power. If this is a passive power, the effects stack.

Powers are either active or passive:
* Passive powers apply at all times.
* Active powers must be activated.

Powers either activate entirely or not at all. 
Eg *Push* can't be activated unless there is an empty space behind the pushed unit.
Eg *Push* with *Beam* can't be activated unless there is an empty space behind *every* pushed unit.

If a power says 'target unit', by default this is the unit directly in front of you. But some passive powers may modify the target.
If there is more than one target-modifying passive on a unit, take the union of all the targets.
Eg with *Beam* and *Spin* target all units on the line ahead *and* all adjacent units.

When a unit is killed, remove it from the board and discard all its powers and tokens.

## miscellanea

Whenever there is a decision to be made (eg which power activates first at the end of the turn, which target unit is targetted first, which power is removed when a unit would have too many powers) it is made by the current player however they prefer. You never make decisions during the other players turn.

Only friendly units can exist in your home space. Any power that would cause a neutral or enemy unit to move into your homespace doesn't work.

A unit is 'outnumbered' if there are more adjacent enemies than allies (including the unit itself).
TODO clarify how to count different teams

Connected units are adjacent units, units adjacent to those units, etc.

'Move' vs 'be moved'. If the movement power belongs to this unit, this unit moved. If the movement power belongs to another unit, this unit was moved.

If the *Flag* power is ever removed from the board for any reason, immediately:
* Kill any unit in the 'F' space.
* Create a neutral unit in the 'F' space.
* Add the *Flag* power.

If the *Flag* power is moved to a non-neutral unit, that unit does not count as adjacent to the flag.

It's possible for a power to cause multiple units to end up in the same space. In that case they become stacked. Units that are not on top of the stack can't be targetted or activated, but still count towards your unit cap.

## playtesting

climb + liability - need to reword movement powers

homeward + jammer ends the game

should 2nd player have two point advantage?
or should they just counterplay better?

should player choose what to discard?

needed to limit spawn rate to avoid dumpingin first round

ninja + condition kills a lot
killing powers with condition on target are more balanced

trail out of homespace? just doesn't trigger, I guess

F is homespace for neutral units?

chain stun was brutal
Maybe should only remove 1 power from among targets. But more surgical might actually be worse.

Need to print flag zone.
Experimenting with different flag zones. Being able to spawn next to flags is rough.

Have neutral units on the board?

Fast and then die?
Or at least just the same power twice.

First player doesn't draw on their first turn.

Catapult could be straight line?

Remote is weirdly phrased.
Maybe should just pick targets as if?

Check which powers we've ignored and see if they can be more fun.

Maybe token cards should have max token count. Eg click-click.

Seeing cards coming helps a bit.

Should flag zone just be middle line?

Maybe units should be smaller so they can rotate.

Can chain split. Maybe can't give split away? Otherwise starting the game with split would be brutal.

A lot of powers need clarifying for multiple targets.

Vacuum doesn't need adjacent to flag - hard to trigger already.

If can't draw from deck, don't need discard, and can keep cards face up so no memory.