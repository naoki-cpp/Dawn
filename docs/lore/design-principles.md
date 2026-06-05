# Lore Design Principles

## Why This Document Exists

Every piece of lore added to Dawn must answer to something higher than taste or mood. This document is that higher authority. It records the *reasons* behind lore decisions so that future contributors — human or AI — can evaluate new ideas without having to reverse-engineer the philosophy.

When a new lore element is proposed, check it against this document first. If it cannot answer the questions in §5, it does not belong here.

---

## 1. Why Post-Collapse?

### The Narrative Opportunity

A living, intact civilization has answers. It has institutions, hierarchies, explanations, experts. Players in an intact civilization inherit a world that is already interpreted for them. There is less room for the player to *matter*.

A post-collapse civilization has gaps. It has questions no one alive can answer. The wreckage of the old world is present, readable in fragments, but the people living in it are making do — using things they did not build, following customs they did not invent, fighting over things they only partially understand.

This is where players have agency. Not because the game gives them power, but because the world *needs* people who are willing to act without certainty.

### Specific Narrative Opportunities the Collapse Creates

**Archaeology as gameplay.** Ancient infrastructure still functions. Abandoned stations, dormant relay networks, derelict ships with intact logs. Discovery is not just flavor — it changes what a player can do.

**Legitimacy is contested.** In an intact civilization, power structures are settled. In a post-collapse world, every claim to authority — "we are the rightful heirs of the old civilization," "we are the only ones who understand this technology" — is a political position, not a fact. This creates natural conflict without requiring a designated villain.

**Knowledge is asymmetric.** One cluster knows something another does not. Information — not just resources — is worth fighting over.

**The moral weight of survival.** When civilization collapsed, people made choices. Some of those choices were heroic. Many were not. The descendants of survivors carry those histories, and those histories shape how they see strangers.

---

## 2. How "Fragmented Universe" Supports the Architecture

Dawn is designed for player-hosted servers. Each server is a self-contained cluster — a pocket of populated star systems with no reliable communication to the outside.

This is not a compromise. It is the premise.

### The Lore Supports the Architecture

In-universe, faster-than-light communication was lost in the collapse. Clusters that survived became isolated not by choice but by physics. What players experience on one server is canonically happening in a different pocket of the same broken universe. Neither server knows the other exists. Neither is wrong.

This means:
- There is no canonical "correct" state of the universe across servers
- A player-run server is not a degraded experience — it *is* the experience, as the lore describes it
- No server is authoritative over another; they are peers, just as the clusters are

### What This Means for World-Building

Every lore element must be true *within a cluster*. It must not require knowledge of other clusters to make sense. Factions, histories, technologies — all of it must function as a self-contained ecosystem.

References to "out there" should exist as rumor, myth, or dead signal — never as confirmed fact. The player should feel that their cluster is the whole world, and feel a specific kind of loneliness when they realize it might not be.

---

## 3. Themes That Run Through All Lore Decisions

### Theme 1: Maintenance vs. Creation

The people of the present can *maintain* things they did not build. They can keep a reactor running, fly a ship manufactured three hundred years ago, read an instruction manual written in a dialect no longer spoken. What they cannot do — or cannot do reliably — is build new things of the same quality.

This creates a civilization that is simultaneously advanced and fragile. Technology is not magic. It is a set of skills, and the chain of transmission broke.

Consequences:
- Old ships are more valuable than new ones (new ships are built from salvage and approximation)
- Knowledge of how something *actually works* is rare and socially powerful
- Factions that control manufacturing infrastructure have leverage even if they are weak militarily

### Theme 2: The Weight of the Record

Because Dawn uses event sourcing, every action in the game is permanently recorded. The lore should reflect this: in-universe, the pre-collapse civilization was obsessed with logging. Everything was recorded — ship movements, resource transfers, communications, even personal medical data.

The collapse did not destroy these records. It scattered them. Fragments survive: corrupted, partial, out of sequence. Reading the old record is possible; understanding it is not.

This means the past is present but illegible. Players live in a world where the evidence of what came before is everywhere, but the context to interpret it is mostly gone.

### Theme 3: The Legitimacy of Violence

People in Dawn fight. The lore must answer: *why do people fight here, specifically?*

The answer is not scarcity alone — scarcity creates pressure, but it does not determine who fights whom. The answer is that in a post-collapse world, the institutions that adjudicated conflict are gone. There is no neutral arbiter. There is no higher power. There are only agreements between groups, enforced by the credible threat of violence.

Fighting is not the default. It is what happens when no other mechanism works. Good lore should make players feel the weight of that: the moment before a fight is not just tactical, it is political and moral.

### Theme 4: Grim But Not Hopeless

The setting is not nihilistic. People are still building things. They have children. They celebrate harvests and mourn the dead. The collapse was a catastrophe, but the people who survived it were not passive. They made choices that preserved *something*.

The tone should feel like: *we lost a great deal, and we know it, and we are still here.* Not: *everything is ash and nothing matters.*

---

## 4. What Questions Every Lore Element Must Answer

These five questions must be answerable for every significant concept introduced:

1. **What is its origin?** Where did this come from? Pre-collapse inheritance, post-collapse innovation, or something that emerged from the gap between them?

2. **How did it change society?** Did it create new hierarchies, dissolve old ones, give some people leverage over others?

3. **How did it change economics?** What became more or less valuable? Who controls it? Who needs it?

4. **How did it change warfare?** Did it change who fights, how they fight, what they fight over?

5. **How did it change daily life?** What is it like to be an ordinary person in a world where this thing exists?

If a lore element cannot answer all five, it is either too small to be in the lore (it belongs in flavor text) or it is not fully developed yet.

---

## 5. What Lore Must Never Do

- **Copy.** No EVE Online factions, no Warhammer 40K aesthetics, no Star Wars resonance. If a concept can be summarized as "it's like X from Y," it needs to be reworked until it cannot.

- **Assign permanent villainy.** No faction is purely evil. Every group in this universe has a reason for existing that makes sense from the inside. If a player wants to join a faction, they should be able to find something to believe in.

- **Answer everything.** The collapse should have unanswered questions. The old civilization should have aspects that are still not understood. Mystery is a resource; spend it slowly.

- **Contradict the architecture.** Lore must not imply features the game does not have: no real-time communication across clusters, no universal trade markets, no permanent faction territory that a player-hosted server cannot contain. The lore is downstream of the technical reality.

- **Reward passivity.** Every lore-justified game mechanic must create decisions. A resource that just sits there and generates income is anti-thematic. Resources must be contested, defended, traded. See game-design.md §5.
