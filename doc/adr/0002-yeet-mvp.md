# 2. Yeet MVP

Date: 2025-12-14

## Status

Accepted

## Context

I figure I should write out my overall thoughts on this silly syncer and what possessed me to even bother spending time on an already solved problem. Hence some architecture design records are in order. This is the "MVP" rationale and implementation plan (as of writing).

### Why?

Well I want to be able to have a sync tool that has the following characterstics:
- The tool should NEVER "lose" use data in files or when there is a conflicting change between sync sources, aka I don't want .conflict file turds all over if the tool thinks things are off. If that is the case the tool should simply bubble that up to the user for conflict resolution.
- Edit crap locally in git repos, but let the data flow elsewhere as well aka edit locally in emacs, maybe do builds remotely etc...
- Sync data one or two way between a source and destination dir locally or remotely.
- Have a work stealing/priority queue type algorithm to eliminate syncing of large files blocking updates to more recent or smaller files. Aka reduce head of queue blocking for data.
- Have a simple interface to query the sync status to perform actions remotely or locally
- Have a simple interface to handle conflicts arising from conflicting changes between sync areas
- The tool should start doing work as soon as it finds stuff to do. Not basically sit there and scan a dir and then do nothing while it finishes scanning a dir with say 2 million files.
- Tool should "hurry up and wait" aka do as much as possible as fast as it possibly can and then when nothing is happening do as little as possible. Preferably not busy wait.

The goals from the work stealing point down are the real reasons for this tool.

Ultimately what I'm trying to implement is something akin to this very old usenix paper:
<https://lingfenghsiang.github.io/docs/DSync.pdf>

While the approach there isn't exactly feasible the overall goals here should be doable without anything super weird. But with EBPF it seems like the overall approach taken recording syscall write offsets and related data to reduce overall network and disk i/o for syncing data between nodes should be doable today.

### Existing alternatives

#### Mutagen

<https://mutagen.io>

<https://github.com/mutagen-io/mutagen>

This is a huge inspiration to the overall tool in addition to dsync.

Pros:

- Single binary makes for very simple install/usage.
- No daemon needed, will ssh to the remote system and copy its own binary and run it remotely * Stealing this approach for sure.
- Configuration is generally through the cli itself.
- Generally simple approach to syncing.
- Uses ssh for authentication/encryption between nodes.
- Can copy data to/from containers as well which is an interesting idea.

Cons:

- Acquired by docker a while back, been ~10 months since any update. My worry is its becoming corporate abandonware.
- Gihugic files block recent updates
- Files that are being written to are copied over wholesale, doesn't really try to minimize data transferred between sync targets.
- The user experience from the cli is... atrocious to put it lightly, it makes it too difficult to get at internal state you might want as a user.
- There isn't a great way to query status without using json output and parsing it and doing other crazy machinations.
- It tends to wake up every 10 seconds by default, causing things like power saving for things like cstates/pstates to get undone.
- Behavior when there are conflicts in files between hosts is not ideal, this can cause other files to not be synced until the conflict is resolved.
- The golang ssh library... sucks, I ran into some very weird issues trying to sync directly to my synology via ssh <https://github.com/mutagen-io/mutagen/issues/495>

Overall mutagen is a really cool tool and I'm not knocking it at all. Its just not the tool I want entirely in the realm of user experience and ability to query it for internal state for things like ci/etc...

Along with the approach dsync took, I'm going to steal a lot of approaches taken by mutagen for overall approach in yeet.

#### Syncthing

<https://syncthing.net>

<https://github.com/syncthing/>

Pros:

- Sortof dropbox but open source.
- Has a cool web gooey you can use to keep track of things.
- Generally works across systems, with caveats around things like synology never got that working.

Cons:

- Where to begin, first off it really can't sync data in git clones. They note this and in spite of it not being what you "should" do its not a huge issue as long as the index file is kept in sync between nodes. With a note of git index files keep things like the inode number of files it looks at so you lose caching between nodes when you do any git operation. Not a huge issue I have found.
- EVERY SINGLE CONFLICT it detects generates a conflict turd file. Trying to figure out which is right isn't easy and for binary files god help you trying to figure out which is the likely one you want.
- As with mutagen getting at data like "is everything synced" programmatically/via the cli is not exactly a pleasure.
- The configuration is in xml, this does not spark joy.
- Overall while it kinda works, it did break git repos I synced. Backups are critical. But the amount of conflict file turds left for basic sync conflicts that it should be able to figure out, aka updated file on nodes b and c, but node a was down and had an older version of the file should sync to the new quorum not construct a conflict turd file and not keep the file in sync. These problems are largely why I abandoned syncthing entirely.

#### rsync + say inotify tools etc...

Not linking to rsync source here but this would be "custom" hacky tooling/solutions around things like rsync.

Pros:

- Platform agnostic, reuses existing tooling.
- Possible to get *close*...ish to the ideal but takes a lot of tooling on top that is... unpleasant to maintain/validate.
- Mostly platform agnostic as you can setup rsync daemons as targets for syncs.

Cons:

- All the custom tooling, shell etc... is ass to maintain over time.
- rsync suffers a lot from the head of queue large files blocking transfer of other data.
- rsync protocol version issues between systems makes for a lot of "weird" af issues that are near impossible to debug even when you aren't using daemon mode.
- All the tooling/code for things like inotify/kqueue etc... to do things like file watching needs to be dealt with on every platform.
- Trying to transfer chunks of files is... unpleasant.
- When you need to sync many small files, rsync is.... glacially slow as molasses as it basically stat()'s, etc... everything serially for each file. It spends most of its time idle waiting on syscalls vs batching work even when it is syncing from one dir to an entirely new dir.

Generally rsync is FINE but the amount of work to get a general tool that works across platforms transparently is even more of a PITA than the other aforementioned tools.

### yeet mvp goals

So instead of trying to work around existing tool issues I figure its "easier" (this is a lie to myself too I knoooooow) to "just" build my own tool with my goals in mind.

General MVP goals:

- yeet should be a single binary too, note while I am going to build binaries to be as static as possible per platform. I will *not* be building a *fat* binary that includes binaries for other platforms.
- yeet should use ssh and be able to handle syncing between nodes over ssh and ssh forward/jump hosts transparently.
- yeet should work "fine" for platforms such as synology where I neither control the linux OS, nor its package manager. In cases where there is no local daemon running behavior should be largely like mutagen, copy appropriate binary remotely exec it, setup sockets between nodes and off to the races.
- yeet should be able to sync one way between targets like mutagen to have copies of data/dirs as appropriate with overwrites of any changes on the destination side etc...
- yeet should have a somewhat easy/simple cli interface that is "unix" like and doesn't require parsing json for what should be basic operations. Aka resolving conflicts programmatically, query for if conflicts exist, what is/isn't in sync. etc..
- For an mvp I want to have syncing in a star topology functional, aka assuming a primary source updates to all leaves should run though the source. Future goal is to figure out a way to have more bittorrent type behavior where a central source of truth is needed. But thats a very future task.

## Decision

I'll be implementing this in rust using nix as the overall build tooling. I'm the most comfortable wit this so its the most appropriate for my own tools.

### MVP

But to list each initial tool and why:

- rust: While not as easy to do cross compilation (and nix/crane helps here so whatever) I can build static or as close to possible to static for all platforms I might care about. I'd definitely prefer zig's build tooling here or golang.
- The main reason for rust is <https://github.com/aya-rs/aya> for future ebpf support that I can compile straight into the binary directly. No libbcc or whatever needed to support kernels that can do the work. I'm sure this will be a rat hole but I do want static binaries to "just work" when its possible. And quick tests of aya seem to work with older kernels for things like write() calls.
- nix cause well, reproducible builds are nice and make development tooling easy/consistent. Plus I've used it for almost a decade. May make contributions more fun in future but thats a scaling issue. Developers should have no real issue with this requirement. It can be avoided too if someone wants, they'll just be replicating all the dependency work in flake.nix.
- Also main reason for rust is it has rust only ssh libraries so that I don't need to depend at all upon ssh locally or remotely for establishing connections between nodes. <https://docs.rs/russh/latest/russh/> is the current crate I'm abusing that seems to work connecting from a nix/macos machine to/from my synology.
- As I've been on a kick to learn ECS systems, <https://bevy.org> is what will be the backbone of the "control-plane" of the application/daemon. Using a video game engine is "weird" but the tick based approach is close to what I was thinking of anyway and with a bit of control theory automatic conflict resolution should be doable without weird crap like paxos/etc...
- Other reason for abusing bevy: with Entities/Components I can re-use existing libraries like <https://github.com/cBournhonesque/lightyear> to handle syncing data to/from systems/daemons. Note I think grpc will be used for all file content data and this cover other non content syncing.
- Readme driven development. I want the user experience to be easy to use, so driving the implementation from a readme first POV so I don't create something crazy to use. Aka should be an easy/intuitive...ish interface for "is everything synchronized?" like yeet sync status returns 0 if synced 1 if not. With details output in an easy way to use existing cli tools like grep awak etc.. without having to parse json from a command line tool.
- And the final reason for nix, I'll reuse the nixpkgs nixos vm test framework to work through integration tests to define behavior of everything in positive and negative use cases. I want this tool to be known to behave in a consistent way in good and bad scenarios.
- Should generally be "faster" than cp or rsync locally and remotely when copying new directories or syncing data.
- initial implementation needs to be able to support me editing on my laptop and having syncing of data through my home gateway to other internal destinations via ssh. As well as syncing directly when in the same network. Basically v0 implementation should be able to pick between disparate routes from a->b and use the fastest or whatever is available as appropriate based on likely RTT between the routes. Local sockets should always be faster when inside the internal network compared to going through a jump or ssh forwarded connection.

### Non/later goals for initial implementation

Note these goals are "future me problems" or "scaling if people actually start to use the dum thing".

- Windows support, I have no clue how to support/test to/from windows. I will build yeet against it to keep myself honest while I build yeet but I don't really have windows systems to test with. Least none appropriate. Some windows developer/user that knows what they're doing can help.
- Container syncing, this would actually be a neat thing to have at some point like what mutagen has but its definitely a future problem.
- Web based ui, I can't make anything user interface related to save my life but would be cool to have a basic web gooey that listens on localhost. Might be easy to have a bevy netcode client in wasm maybe that connects to the locally running daemon? This too might be better done by people that know web gooeys I sure as hell don't.
- ebpf support, as much as I want this in the v0 implementation, I need to have non ebpf syncing working first so no need to prioritize this as of yet.


## Consequences

All decisions in this initial adr will impact the lifecycle of yeet such as language and any tooling etc... While changing these decisions after the fact isn't impossible the overall approach/architecture using bevy and ssh and static binaries/daemons in general will require a lot of planning and may not be easily possible after things are built. That is inevitable however, as my late gramps intimated: something that works even if poorly is preferable to something well designed but non existent. Using a video game ECS may not be the best idea but it actually seems conceptually like a better solution than normal setups in that it gives me a temporal "tick" to operate off of for control theory logic. I can do without the ecs and do the overall work too, but the main selling point of an ECS is its ecosystem, I don't need to reinvent wheels of syncing data like Components between daemons.

If it becomes an issue, and this means after profiling and measuring versus some pie in the sky on high architecture astronauting, I can revisit/refactor/rebuild at that point. I doubt this will become an issue. Bevy ECS Systems are "just" rust functions that operate off of data I stick into the ECS. While the ecs implicitly introduces latency based off of engine tick frequency, it also provides a clean way to operate off of "known good" data and separates out "incoming" data. I also gain a lot of things "for free" by using existing netcode libraries etc... by using bevy and its ecosystem. While I'd love to tackle this stuff, its really outside of what yeet itself is intended to be. The overall sync logic alone is complex enough that I'd rather get that tackled and defined before I try building infrastructure like the ECS etc...
