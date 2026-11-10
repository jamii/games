# Rules decisions

The automatic rules (`src/rules.rs`) have to decide things the rules text leaves open.
This lists every such decision, so we can go through them. Each one is a judgement call,
not a claim about what the rules mean. Anything marked **(simplification)** deliberately
does less than the rules ask, usually to avoid asking the player pointless questions.

Tests for most of these are in `src/rules/tests.rs`.

## Resolving powers

- **All or nothing.** Every requirement of a power has to hold, for every target, or the
  power can't be used at all. With several targets that means every target has to qualify:
  *Push* needs a space behind every pushed unit (as the rules say), *Dogpile* needs every
  target outnumbered, *Backstab* needs a friend behind every target, *Stun* needs every
  target to have a power, *Convert* needs every target neutral, *Recall* needs every target
  friendly.
- **A power that wouldn't do anything can't be used.** For example *Walk* when blocked,
  *Neutralize* with no powerless units, or *Mine* away from the flag. You can still activate
  the unit just to turn it.
- **Several targets are handled one at a time, nearest first** (ties: the unit that came
  into play first). The rules say the current player chooses the order.
  **(simplification)** The order rarely matters, and asking every time would be tedious.
- **Powers that only make sense with one target** (*Swap*, *Jump*, *Climb*, *Circle*,
  *Trade*, *Trigger*, *Possess*): if several units are targeted, the player picks one.
  Only targets the power can actually work on are offered.
- **Choices are offered only if they can be completed.** The player is never offered an
  option that leads to a dead end. A choice with a single option is made automatically.
- An activation that doesn't use a power has to turn the unit, otherwise it would do nothing.

## Turn structure

- "Draw 2 cards" works like the setup draft: take a faceup card or the top card of the
  deck. Taking a faceup card works the same way everywhere (the draft, the draws at the
  start of a turn, and *Mine*): the cards to its right slide left, and a new card is drawn
  on the right.
- When the deck runs out, the discard pile is shuffled to form a new deck.
- Spawned units face the far side of the board. The player can turn them when they activate.
- Applying a power to a full unit means choosing which power it replaces. A power can only
  be replaced when the unit already has 3. Replacing a free power with the same free power
  isn't allowed, because nothing would change.
- End of turn, in order: end-of-turn powers (*Heavy*, *Stinky*; the player picks the order
  if there are several), then scoring, then the win check.
- Scoring is automatic: you score if your count next to the flag is higher.
- The flag can be activated by either player, but only if it has an active power (from the
  advanced setup option). Each player gets its own activation of it on their turn.
- End-of-turn powers on neutral units (for example the flag's extra power) happen at the end
  of every turn, since neutral units have no turn of their own.

## Units, stacks and home spaces

- **Buried units** (not on top of a stack) are inert. They can't act or be targeted, aren't
  adjacent to anything, don't score, and their passive powers do nothing. They still count
  towards the unit cap. The flag still works while buried, and it scores from its own space.
- **Home spaces:** a unit can never end a move in another player's home space, and neutral
  units can never enter a home space. This applies to every kind of move, including the
  user's own moves.
- **Six units:** whenever a player gets a sixth unit (spawning, *Split*, *Convert*), they
  immediately pick one of their units to kill, which may be the new one. This kill isn't by a
  power, so *Phoenix*, *Pacifist* and *Lifeguard* don't apply. If the new unit survives, it
  takes the freed label.
- **Spawning** is only allowed in an empty home space.

## Targets

- With no targeting powers, the target is the top unit directly in front. With targeting
  powers, the default target is replaced by the union of what each one targets. The user is
  never its own target.
- Only units on top of a stack can be targeted.
- *Reach*: the unit two spaces directly ahead ("one space away" means one space in between,
  as with *Halo*). Whatever is in between doesn't matter.
- *Spin*, *Chain*, *Stalker*, *Spotlight* and *Bully* use unit adjacency, so *Bridge* and
  *Telepath* count.
- *Beam* and *Six-shooter* follow straight lines. *Beam* takes every unit to the edge of the
  board and isn't blocked; *Six-shooter* takes the first unit in each of the six directions.
- *Halo*: every unit exactly two spaces away, in or out of a straight line.
- *Liability*: the user's other friendly units. A neutral unit has none.
- *Vengeance*: enemy units that killed one of your units during the previous turn and are
  still on the board. The killer is the unit whose power did the killing (for *Explosive*,
  the exploding unit; for *Spiky*, the spiky unit).
- Passive powers that say "target unit" (*Grip*, *Jammer*, *Weak*, *Lifeguard*, *Friendly*,
  *Explosive*) use the same targets as the unit's active powers, worked out continuously
  from where it is and which way it faces.
- To avoid a loop (*Bully* depends on outnumbering, which depends on *Weak*), *Weak* and
  *Friendly* work out their own targets while ignoring *Weak* and *Friendly*.

## Adjacency, outnumbering and scoring

- *Bridge*: two units count as adjacent if they're both next to the same *Bridge* unit.
  This doesn't chain through further bridges.
- *Telepath*: adjacent to every unit with the same owner. Neutral units have no friends, so
  *Telepath* does nothing on them.
- Directional things (*Push*, *Pull*, *Jump*, *Backstab*, *Stinky*, *Twitchy*, "in front")
  use real board geometry, not *Bridge*/*Telepath* adjacency.
- **Outnumbered** (the rules have a TODO here): for a player's unit, allies are itself plus
  adjacent units on its side, and enemies are adjacent units on the other side. Neutral
  units count for neither side. For a neutral unit, allies are itself plus adjacent neutral
  units, and enemies are all adjacent player units. A neutral unit befriended (*Friendly*) by
  one player counts as that player's unit. One befriended by both is never outnumbered.
  *Strong* and *Weak* change the counts (a unit can't count as less than 0).
- Scoring counts units adjacent to the unit holding the *Flag* power, with *Strong*,
  *Weak*, *Friendly*, *Bridge* and *Telepath* applied. The flag holder itself never counts.
- *Friendly*: a neutral unit befriended by a player counts for them for scoring and
  outnumbering. Both players can befriend the same unit.

## Movement

- The user moves itself; any other unit moved by its power "was moved" by the user.
- *Grip* stops a unit moving itself. *Stubborn* stops it being moved by others (including by
  *Leader*, *Heavy* and *Stinky*). Both make the whole power fail.
- *Flag*: the flag holder can't move more than one space in total per turn, counting the
  distance of every move. A power that would break this fails.
- Moves happen one unit at a time unless the power moves several units together (*Swap*).
  Triggers (*Spiky*, *Leader*, *Trail*, *Twitchy*) happen right after each move, in that
  order.
- *Leader*: "an adjacent unit" means a unit adjacent to the leader after it has moved. The
  player picks which one, and one must follow if any can. Extra copies of *Leader* do
  nothing more.
- *Trail* doesn't trigger when leaving a home space (from the playtesting notes). If a unit
  has both *Leader* and *Trail*, *Leader* goes first, so the space is filled by the follower
  instead.
- *Spiky*: the unit still gets moved, then the unit whose power moved it dies.
- *Twitchy*: triggers whenever an enemy becomes adjacent (geometrically), whether the enemy
  or the twitchy unit moved, or a unit was created. With several new enemies, the player
  picks which one to face.
- *Phoenix* and *Pacifist* put the unit back in a home space; this isn't a move.

## Killing and powers leaving the board

- *Lifeguard* stops kills by powers. The protected unit survives, but the power still counts
  as used (it isn't illegal just because one target is protected).
- *Phoenix* and *Pacifist* replace death: the unit keeps its powers and tokens and doesn't
  count as killed (so *Explosive* doesn't go off). The player picks which empty home space.
  With no empty home space the unit dies. Neutral units have no home, so they just die.
- Kills by the rules (the six-unit cap, and clearing F for a new flag) aren't kills "by a
  power".
- Units killed together are worked out together. *Lifeguard*, *Phoenix* and *Pacifist* are
  checked before any of them die, and explosions happen after.
- *Explosive*: when the unit dies (by any means), its current targets die too (a kill by a
  power). Explosions can chain.
- **Where removed powers go**, checked in this order: the *Flag* power is never moved
  anywhere and triggers the new-flag rule instead. Then *Vacuum* (to a vacuum unit next to
  the flag; the player picks if there are several). Then free powers just disappear. Then
  *Recycle* (to the hand of the player whose unit lost it). Then the discard pile.
- *Vacuum* doesn't catch powers taken off the vacuum unit itself. A power knocked off a
  full unit to make room skips *Vacuum*, so two full vacuums can't pass powers back and
  forth forever.
- *Recycle*: "return to the deck" is read as "discard", since removed powers go to the
  discard pile.

## Powers

- *Run*: moves to the space two ahead. Only that space has to be empty; it can jump over a
  unit in between.
- *Rush*: moves as far as it can and stops before a unit, the edge of the board, a home space
  it can't enter, or (for the flag) its movement limit. It must move at least one space.
- *Push*: each target must be in a straight line from the user, and moves one space directly
  away from it. Then the user moves into the space the nearest target left (stacking if
  something else ended up there). Even a distant target (*Reach*, *Beam*) is pushed this way,
  and the user then jumps into its old space.
- *Pull*: the nearest target must be in a straight line. The user steps one space directly
  away from it (that space must be empty), then every target moves into the user's old space,
  stacking up.
- *Lift*: each target moves to an empty space next to it, of the player's choice.
- *Jump*: lands one space beyond the target, in a straight line from the user.
- *Climb*: the user moves onto the target's space, on top of the stack.
- *Catapult*: works from the user's own home space, to any empty space except the enemy's
  home spaces.
- *Bungee*: to an empty space in the user's own home. Neutral units can't use it.
- *Burrow*: to an empty space with no unit (other than the user) next to it.
- *Heavy*: units exactly two spaces away move to a space next to both them and the heavy
  unit (the player picks if there are two). Units already adjacent can't get any closer.
  Units are handled one at a time, and each moves only if its space is still empty. Each copy
  of *Heavy* does this once.
- *Stinky*: geometrically adjacent units are pushed one space directly away, if that space
  is empty. Each copy does this once.
- *Trade*: the user gives one of its powers (*Trade* itself included) for one of the
  target's. The target can't already have the power it's given. Free powers stay free.
- *Stun*: the player picks which power each target loses.
- *Trigger*: uses one of the target's active powers as if the user had it (so the user's
  position, facing and targeting powers apply). It can't borrow *Trigger* or *Mystery*, and
  can't borrow a power that's forbidden this turn.
- *Mystery*: the card from your hand takes *Mystery*'s place, and *Mystery* is discarded
  (or caught by *Vacuum*/*Recycle*). It can't be used to apply a forbidden card.
- *Fast*: each copy gives one more activation per turn.
- *Click Click* and *Nuke*: tokens stay on the unit, and they're lost when it dies.
  "Remove a token and kill" is only offered when there's a token (or 3, for *Nuke*) and
  something to kill. *Nuke* kills every unit, buried ones and itself included.
- *Neutralize*: kills every powerless unit, including buried ones.
- *Banish*: each target goes to an empty space in its owner's home (the player picks which).
  Neutral units can't be banished.
- *Split*: the new unit goes in the space in front, faces the same way, and belongs to the
  same player. The player picks any subset of powers to give it, possibly none.
- *Convert*: the unit keeps its powers, tokens, position and facing.
- *Mine*: draws like the start of the turn (faceup or deck), into the hand of the player
  using it.
- *Forbid*: any power except the free powers and *Flag* can be named. It lasts until the
  end of the opponent's next turn, and blocks applying the card from hand, using it,
  borrowing it with *Trigger*, and *Mystery* turning into it.
- *Recall*: the target's cards go to the user's owner's hand (free powers and *Flag* don't),
  then it dies. Since its powers are gone first, *Phoenix* can't save it, but *Pacifist* can
  (it goes home powerless).
- *Possess*: the user's powers are added to the target one at a time, and the player picks
  replacements if it fills up. Then the user dies (by a power).
- *Remote*: only changes where targets are measured from. The playtesting notes suggest
  "pick targets as if". It works from the space of any friendly unit on top of a stack, using
  the user's facing. Distances and directions for the effect itself (pushing away, jumping
  over, the space behind) are still measured from where the user really is. It only applies
  to powers that target units.

## Not decided yet

- **A *Split* chain gives unlimited actions in a turn.** A unit created this
  turn hasn't been activated yet, so it can act straight away. A unit with
  *Split* can give the new unit *Split*. If the player already has five units,
  they kill the old one, and the new unit (one space further forward) splits
  again. The rules as written allow this forever, and the bots found it: one
  game had over 500 splits in a single turn. Options: new units can't be
  activated on the turn they're created, or *Split* can't give itself away.
  (Bots cap themselves at 16 actions per turn, which hides this in experiments.)

- **Powers that move the flag look too strong.** The *Flag* card lets the flag be
  moved one space per turn, so *Swap* (swap places with it), *Heavy* (pull it in at
  the end of your turn), *Pull* and *Lift* drag it towards your units; with either card in hand a player
  wins about 30 percentage points more often (see BOT.md, "Cards"). Options: no power
  can move the flag, or only its holder can. Measured with `selfplay --fixed-flag` (no
  power can move it): Swap drops to +7, Heavy to 0 and *Pull* (also +30) to −3, in line
  with other cards.
  *Trade* is the same problem by another route (worth +30 to the stronger bot): it can
  take the *Flag* power itself, making the trader's unit the flag. With the *Flag* power
  not tradeable as well (also part of `--fixed-flag`), Trade drops to +2.

- **Do buried units count at the flag?** We decided they don't (see "Units, stacks and
  home spaces"), but the rules text doesn't say. It matters: with that reading *Climb* is
  worth +20 percentage points in hand, because climbing onto an enemy next to the flag
  takes it out of the count (see BOT.md, "Cards").

- Rule details from the playtesting notes that aren't in the rules text, like "first player
  doesn't draw on their first turn" and the second player's starting points, are options in
  the **New game** dialog.
- A power added to powers.md that isn't listed in `Effect` (in `src/data.rs`) has no
  automatic effect: as an active power it can't be used, and as a passive power it does
  nothing.
