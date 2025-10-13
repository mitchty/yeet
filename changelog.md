# Changelog

All notable changes to this project will be documented in this file.

This project adheres to [Semantic Versioning](https://semver.org).

<!--
Note: In this file, do not use the hard wrap in the middle of a sentence for compatibility with GitHub comment style markdown rendering.
-->

## [Todo]

There is zero grpc/ssh interconnection stuff setup. Waiting for lightyear to be ported to Bevy 0.17.x, I'll be abusing its replication initially for inter daemon communication and as well as the cli monitoring setup.

Tons of rpc's to add, like query, remove, cancel etc...

Error handling needs to be figured out, if you try copying say /root to /tmp/etc as a regular user it'll error every tick of the ecs.

Need to implement some sort of backoff/short circuit kinda behavior. But I do want yeet to retry things periodically, whatever was broken might be fixed later on.

Probably want to create a few ADR's for why I decided to implement any of this the way I did. Its not a very conventional approach but I have my reasons/secrets behind the decision.

## [Unreleased]

## [0.0.2] 2025-10-13

Yeet is a very crappy cp command with a grpc interface and zero regard for errors.

## [0.0.1] - 2025-01-20

Just added barebones bevy so I can abuse the ecs there for everything. This version does LESS than the last >.<

## [0.0.0] - 2025-01-19

Right now the binary only does a one shot copy from source dir to target like rsync only without syncing. Its useless for real world usage right now.
