# W3C SCXML 1.0 IRP tests

Survey of the W3C SCXML 1.0 Implementation Report Plan tests (<https://www.w3.org/Voice/2013/scxml-irp/>, version of 10 March 2015) against decree's SCXML subset (`docs/0.5-spec.md`, section 5). Ticket M3.4.

The manifest (`https://www.w3.org/Voice/2013/scxml-irp/manifest.xml`) and every test file it names were downloaded on 2026-10-02: 200 tests, 206 `.txml` documents and 5 `.txt` resources. Each test was checked for every SCXML element and attribute it uses, including the `conf:` placeholders the IRP substitutes per data model (`conf:id`, `conf:expr`, `conf:idVal`, … stand for `<datamodel>` locations, expressions and conditions; `conf:targetpass` and `conf:targetfail` stand for `target="pass"` and `target="fail"`; `conf:pass` and `conf:fail` are final states).

A test could be ported if it uses nothing outside the subset, read this way:

- `<state>`, `<final>`, `initial`, `type="internal"`, `done.state.<id>` and transitions with one event descriptor and one target are in the subset.
- An eventless transition with a target and no `cond` is decree's pass-through `done` (section 5, Kinds of state).
- A missing `initial` attribute is written as an explicit `initial:` naming the first child in document order, because decree requires `initial`. The exception is test 355, which tests that default itself.
- Executable content (`<raise>`, `<send>`, `<assign>`, `<log>`, `<if>`, `<foreach>`, `<script>`, `<cancel>`) is outside the subset even inside `<onentry>` or `<onexit>`: decree's `onentry` and `onexit` hold script names, never executable content.

**Result: no test falls inside the subset, so none is ported.** The IRP checks behaviour by raising or sending events, by data model values, or with `*` catch-all transitions, which decree does not have. Without them, the only events are `done.state.<id>` and eventless transitions, and the one test limited to those (355) needs the default initial state. decree's composition semantics are tested instead by the `composition_*`, `internal_*`, `nested_final_*` and `waiting_*` tests in `src/interpreter.rs`, on the fixtures in `tests/fixtures/machines/step/`.

A ported test would be `<test id>.yml` here: a decree machine whose run ends in the final state `pass`. `src/interpreter.rs` runs every `*.yml` in this directory and checks that the table below lists every other test.

## Tests not ported

Each row gives the test id, the SCXML section it covers, its conformance level, and the features it needs that are outside decree's subset. "manual" means the IRP marks the test as checked by hand.

| Test | Section | Conformance | Needs |
| --- | --- | --- | --- |
| 144 | 4.2 | mandatory | `<raise>`, event wildcard `*` |
| 147 | 4.3 | mandatory | `<raise>`, `<if>`, `<datamodel>`, event wildcard `*` |
| 148 | 4.3 | mandatory | `<raise>`, `<if>`, `<datamodel>`, event wildcard `*` |
| 149 | 4.3 | mandatory | `<raise>`, `<if>`, `<datamodel>`, event wildcard `*` |
| 150 | 4.6 | mandatory | `<raise>`, `<foreach>`, `<datamodel>`, event wildcard `*`, several transitions for one event |
| 151 | 4.6 | mandatory | `<raise>`, `<foreach>`, `<datamodel>`, event wildcard `*`, several transitions for one event |
| 152 | 4.6 | mandatory | `<raise>`, `<foreach>`, `<datamodel>`, event wildcard `*`, several transitions for one event |
| 153 | 4.6 | mandatory | `<assign>`, `<if>`, `<foreach>`, `<datamodel>`, several transitions for one event |
| 155 | 4.6 | mandatory | `<foreach>`, `<datamodel>`, several transitions for one event |
| 156 | 4.6 | mandatory | `<assign>`, `<foreach>`, `<datamodel>`, several transitions for one event |
| 158 | 4.9 | mandatory | `<raise>`, `<datamodel>`, event wildcard `*` |
| 159 | 4.9 | mandatory | `<send>`, `<datamodel>`, several transitions for one event |
| 172 | 6.2 | mandatory | `<send>`, `<assign>`, `<datamodel>`, event wildcard `*` |
| 173 | 6.2 | mandatory | `<send>`, `<assign>`, `<datamodel>`, event wildcard `*` |
| 174 | 6.2 | mandatory | `<send>`, `<assign>`, `<datamodel>`, event wildcard `*` |
| 175 | 6.2 | mandatory | `<send>`, `<assign>`, `<datamodel>`, event wildcard `*` |
| 176 | 6.2 | mandatory | `<send>`, `<assign>`, `<param>`, `<datamodel>`, event wildcard `*`, several transitions for one event |
| 178 | 6.2 | mandatory | manual, `<send>`, `<log>`, `<param>`, `<datamodel>`, event wildcard `*` |
| 179 | 6.2 | mandatory | `<send>`, `<content>`, `<datamodel>`, event wildcard `*` |
| 183 | 6.2 | mandatory | `<send>`, `<datamodel>`, several transitions for one event |
| 185 | 6.2 | mandatory | `<send>`, `<datamodel>`, event wildcard `*` |
| 186 | 6.2 | mandatory | `<send>`, `<assign>`, `<param>`, `<datamodel>`, event wildcard `*`, several transitions for one event |
| 187 | 6.2 | mandatory | `<invoke>` of an SCXML session, `<send>`, `<content>`, `<datamodel>` |
| 189 | C.1 | mandatory | `<send>` |
| 190 | C.1 | mandatory | `<send>`, `<raise>`, `<datamodel>`, event wildcard `*` |
| 191 | C.1 | mandatory | `<invoke>` of an SCXML session, `<send>`, `<content>`, event wildcard `*` |
| 192 | C.1 | mandatory | `<invoke>` of an SCXML session, `<send>`, `<content>` |
| 193 | C.1 | optional | `<send>` |
| 194 | 6.2 | mandatory | `<send>`, `<datamodel>`, event wildcard `*` |
| 198 | 6.2 | mandatory | `<send>`, `<datamodel>`, event wildcard `*` |
| 199 | 6.2 | mandatory | `<send>`, `<datamodel>`, event wildcard `*` |
| 200 | 6.2 | mandatory | `<send>`, event wildcard `*` |
| 201 | 6.2 | optional | `<send>`, `<datamodel>`, event wildcard `*` |
| 205 | 6.2 | mandatory | `<send>`, `<assign>`, `<param>`, `<datamodel>`, event wildcard `*`, several transitions for one event |
| 207 | 6.3 | mandatory | `<invoke>` of an SCXML session, `<send>`, `<cancel>`, `<content>`, `<datamodel>`, event wildcard `*` |
| 208 | 6.3 | mandatory | `<send>`, `<cancel>`, `<datamodel>`, event wildcard `*` |
| 210 | 6.3 | mandatory | `<send>`, `<cancel>`, `<assign>`, `<datamodel>`, event wildcard `*` |
| 215 | 6.4 | mandatory | `<invoke>` of an SCXML session, `<send>`, `<assign>`, `<content>`, `<datamodel>`, event wildcard `*` |
| 216 | 6.4 | mandatory | `<invoke>` of an SCXML session, `<send>`, `<assign>`, `<datamodel>`, event wildcard `*` |
| 220 | 6.4 | mandatory | `<invoke>` of an SCXML session, `<send>`, `<content>`, event wildcard `*` |
| 223 | 6.4 | mandatory | `<invoke>` of an SCXML session, `<send>`, `<content>`, `<datamodel>`, event wildcard `*`, several transitions for one event |
| 224 | 6.4 | mandatory | `<invoke>` of an SCXML session, `<send>`, `<content>`, `<datamodel>`, event wildcard `*`, several transitions for one event |
| 225 | 6.4 | mandatory | `<invoke>` of an SCXML session, `<send>`, `<content>`, `<datamodel>`, event wildcard `*`, several transitions for one event |
| 226 | 6.4 | mandatory | `<invoke>` of an SCXML session, `<send>`, `<param>`, `<datamodel>`, event wildcard `*` |
| 228 | 6.4 | mandatory | `<invoke>` of an SCXML session, `<send>`, `<assign>`, `<content>`, `<datamodel>`, event wildcard `*`, several transitions for one event |
| 229 | 6.4 | mandatory | `<invoke>` of an SCXML session, `<send>`, `<content>`, event wildcard `*`, targetless transition |
| 230 | 6.4 | mandatory | manual, `<invoke>` of an SCXML session, `<send>`, `<log>`, `<content>`, `<datamodel>`, event wildcard `*` |
| 232 | 6.4 | mandatory | `<invoke>` of an SCXML session, `<send>`, `<content>` |
| 233 | 6.4 | mandatory | `<invoke>` of an SCXML session, `<finalize>`, `<send>`, `<assign>`, `<content>`, `<param>`, `<datamodel>`, event wildcard `*` |
| 234 | 6.4 | mandatory | `<parallel>`, `<invoke>` of an SCXML session, `<finalize>`, `<send>`, `<assign>`, `<content>`, `<param>`, `<datamodel>`, several transitions for one event |
| 235 | 6.4 | mandatory | `<invoke>` of an SCXML session, `<send>`, `<content>`, event wildcard `*` |
| 236 | 6.4 | mandatory | `<invoke>` of an SCXML session, `<send>`, `<content>`, `<datamodel>`, event wildcard `*`, `<onexit>` on `<final>` |
| 237 | 6.4 | mandatory | `<invoke>` of an SCXML session, `<send>`, `<content>`, `<datamodel>`, event wildcard `*` |
| 239 | 6.4 | mandatory | `<invoke>` of an SCXML session, `<send>`, `<content>` |
| 240 | 6.4 | mandatory | `<invoke>` of an SCXML session, `<send>`, `<content>`, `<param>`, `<datamodel>`, several transitions for one event |
| 241 | 6.4 | mandatory | `<invoke>` of an SCXML session, `<send>`, `<content>`, `<param>`, `<datamodel>`, several transitions for one event |
| 242 | 6.4 | mandatory | `<invoke>` of an SCXML session, `<send>`, `<content>` |
| 243 | 6.4 | mandatory | `<invoke>` of an SCXML session, `<send>`, `<content>`, `<param>`, `<datamodel>`, event wildcard `*`, several transitions for one event |
| 244 | 6.4 | mandatory | `<invoke>` of an SCXML session, `<send>`, `<content>`, `<datamodel>`, event wildcard `*`, several transitions for one event |
| 245 | 6.4 | mandatory | `<invoke>` of an SCXML session, `<send>`, `<content>`, `<datamodel>`, event wildcard `*`, several transitions for one event |
| 247 | 6.4 | mandatory | `<invoke>` of an SCXML session, `<send>`, `<content>` |
| 250 | 6.4 | mandatory | manual, `<invoke>` of an SCXML session, `<send>`, `<log>`, `<content>`, `<datamodel>` |
| 252 | 6.4 | mandatory | `<invoke>` of an SCXML session, `<send>`, `<content>`, `<datamodel>` |
| 253 | 6.4 | mandatory | `<invoke>` of an SCXML session, `<send>`, `<assign>`, `<content>`, `<datamodel>`, several transitions for one event |
| 276 | 5.3 | mandatory | `<invoke>` of an SCXML session, `<param>`, `<datamodel>` |
| 277 | 5.3 | mandatory | `<raise>`, `<assign>`, `<datamodel>`, event wildcard `*`, several transitions for one event |
| 278 | B.2 | optional | `<datamodel>`, several transitions for one event |
| 279 | 5.3 | mandatory | `<datamodel>`, several transitions for one event |
| 280 | 5.3 | mandatory | `<assign>`, `<datamodel>`, several transitions for one event |
| 286 | 5.4 | mandatory | `<raise>`, `<assign>`, `<datamodel>`, event wildcard `*` |
| 287 | 5.4 | mandatory | `<assign>`, `<datamodel>`, several transitions for one event |
| 294 | 5.5 | mandatory | `<donedata>`, `<param>`, `<datamodel>`, several transitions for one event |
| 298 | 5.7 | mandatory | `<send>`, `<donedata>`, `<param>`, `<datamodel>`, event wildcard `*` |
| 301 | 5.8 | mandatory | manual, `<script>`, `<datamodel>` |
| 302 | 5.8 | mandatory | `<datamodel>`, several transitions for one event |
| 303 | 5.8 | mandatory | `<assign>`, `<datamodel>`, several transitions for one event |
| 304 | 5.8 | mandatory | `<datamodel>`, several transitions for one event |
| 307 | 5.9 | mandatory | manual, `<raise>`, `<log>`, `<datamodel>` |
| 309 | 5.9 | mandatory | `<datamodel>`, several transitions for one event |
| 310 | 5.9 | mandatory | `<parallel>`, `<datamodel>`, several transitions for one event |
| 311 | 5.9 | mandatory | `<send>`, `<assign>`, `<datamodel>`, event wildcard `*` |
| 312 | 5.9 | mandatory | `<raise>`, `<assign>`, `<datamodel>`, event wildcard `*` |
| 313 | 5.9 | mandatory | manual, `<raise>`, `<assign>`, `<datamodel>`, event wildcard `*` |
| 314 | 5.9 | mandatory | manual, `<raise>`, `<assign>`, `<datamodel>`, event wildcard `*` |
| 318 | 5.10 | mandatory | `<raise>`, `<assign>`, `<datamodel>`, several transitions for one event |
| 319 | 5.10 | mandatory | `<raise>`, `<if>`, `<datamodel>` |
| 321 | 5.10 | mandatory | `<datamodel>`, several transitions for one event |
| 322 | 5.10 | mandatory | `<raise>`, `<assign>`, `<datamodel>`, event wildcard `*`, several transitions for one event |
| 323 | 5.10 | mandatory | `<datamodel>`, several transitions for one event |
| 324 | 5.10 | mandatory | `<assign>`, `<datamodel>`, several transitions for one event |
| 325 | 5.10 | mandatory | `<datamodel>`, several transitions for one event |
| 326 | 5.10 | mandatory | `<raise>`, `<assign>`, `<datamodel>`, event wildcard `*`, several transitions for one event |
| 329 | 5.10 | mandatory | `<raise>`, `<assign>`, `<datamodel>`, event wildcard `*`, several transitions for one event |
| 330 | 5.10 | mandatory | `<send>`, `<raise>`, `<datamodel>`, event wildcard `*` |
| 331 | 5.10 | mandatory | `<send>`, `<raise>`, `<assign>`, `<datamodel>`, event wildcard `*`, several transitions for one event |
| 332 | 5.10 | mandatory | `<send>`, `<assign>`, `<datamodel>`, event wildcard `*`, several transitions for one event |
| 333 | 5.10 | mandatory | `<send>`, `<datamodel>`, event wildcard `*` |
| 335 | 5.10 | mandatory | `<raise>`, `<datamodel>`, event wildcard `*` |
| 336 | 5.10 | mandatory | `<send>`, `<datamodel>`, event wildcard `*` |
| 337 | 5.10 | mandatory | `<raise>`, `<datamodel>`, event wildcard `*` |
| 338 | 5.10 | mandatory | `<invoke>` of an SCXML session, `<send>`, `<assign>`, `<content>`, `<datamodel>`, several transitions for one event |
| 339 | 5.10 | mandatory | `<raise>`, `<datamodel>`, event wildcard `*` |
| 342 | 5.10 | mandatory | `<send>`, `<assign>`, `<datamodel>`, event wildcard `*`, several transitions for one event |
| 343 | 5.7 | mandatory | `<donedata>`, `<param>`, `<datamodel>`, event wildcard `*`, several transitions for one event |
| 344 | 5.9 | mandatory | `<raise>`, `<datamodel>`, event wildcard `*`, several transitions for one event |
| 346 | 5.10 | mandatory | `<raise>`, `<assign>`, `<datamodel>`, event wildcard `*`, targetless transition |
| 347 | C.1 | mandatory | `<invoke>` of an SCXML session, `<send>`, `<content>` |
| 348 | C.1 | mandatory | `<send>`, event wildcard `*` |
| 349 | C.1 | mandatory | `<send>`, `<assign>`, `<datamodel>`, event wildcard `*` |
| 350 | C.1 | mandatory | `<send>`, `<datamodel>`, event wildcard `*` |
| 351 | C.1 | mandatory | `<send>`, `<assign>`, `<datamodel>`, event wildcard `*`, several transitions for one event |
| 352 | C.1 | mandatory | `<send>`, `<assign>`, `<datamodel>`, event wildcard `*`, several transitions for one event |
| 354 | C.1 | mandatory | `<send>`, `<assign>`, `<content>`, `<param>`, `<datamodel>`, event wildcard `*`, several transitions for one event |
| 355 | 3.2 | mandatory | default initial state |
| 364 | 3.3 | mandatory | `<parallel>`, `<send>`, `<raise>`, several targets on one transition, `<initial>` element |
| 372 | 3.7 | mandatory | `<send>`, `<assign>`, `<datamodel>`, event wildcard `*`, `<onexit>` on `<final>` |
| 375 | 3.8 | mandatory | `<raise>`, event wildcard `*` |
| 376 | 3.8 | mandatory | `<send>`, `<datamodel>`, several transitions for one event |
| 377 | 3.9 | mandatory | `<raise>`, event wildcard `*` |
| 378 | 3.9 | mandatory | `<send>`, `<datamodel>`, several transitions for one event |
| 387 | 3.10 | mandatory | `<history>`, `<send>`, `<raise>`, event wildcard `*` |
| 388 | 3.10 | mandatory | `<history>`, `<send>`, `<raise>`, `<datamodel>`, several transitions for one event |
| 396 | 3.12 | mandatory | `<raise>`, `<datamodel>`, several transitions for one event |
| 399 | 3.12 | mandatory | `<send>`, `<raise>`, event wildcard `*`, several event descriptors on one transition |
| 401 | 3.12 | mandatory | `<send>`, `<assign>`, `<datamodel>` |
| 402 | 3.12 | mandatory | `<send>`, `<raise>`, `<assign>`, `<datamodel>`, event wildcard `*` |
| 403 | 3.13 | mandatory | `<send>`, `<raise>`, `<datamodel>`, event wildcard `*` |
| 404 | 3.13 | mandatory | `<parallel>`, `<raise>`, event wildcard `*` |
| 405 | 3.13 | mandatory | `<parallel>`, `<send>`, `<raise>`, event wildcard `*` |
| 406 | 3.13 | mandatory | `<parallel>`, `<send>`, `<raise>`, event wildcard `*` |
| 407 | 3.13 | mandatory | `<datamodel>`, several transitions for one event |
| 409 | 3.13 | mandatory | `<send>`, `<raise>`, `<if>`, `<datamodel>` |
| 411 | 3.13 | mandatory | `<send>`, `<raise>`, `<if>`, `<datamodel>` |
| 412 | 3.13 | mandatory | `<send>`, `<raise>`, event wildcard `*`, `<initial>` element |
| 413 | 3.13 | mandatory | `<parallel>`, `<datamodel>` |
| 415 | 3.13 | mandatory | manual, `<raise>` |
| 416 | 3.13 | mandatory | `<send>` |
| 417 | 3.13 | mandatory | `<parallel>`, `<send>` |
| 419 | 3.13 | mandatory | `<send>`, `<raise>`, event wildcard `*` |
| 421 | 3.13 | mandatory | `<send>`, `<raise>` |
| 422 | 3.13 | mandatory | `<invoke>` of an SCXML session, `<send>`, `<content>`, `<datamodel>`, several event descriptors on one transition, several transitions for one event, targetless transition |
| 423 | 3.13 | mandatory | `<send>`, `<raise>`, `<datamodel>`, event wildcard `*` |
| 436 | B.1 | mandatory | `<parallel>`, `<datamodel>`, several transitions for one event |
| 444 | B.2 | optional | `<datamodel>`, `cond` outside a router state, several transitions for one event |
| 445 | B.2 | optional | `<datamodel>`, `cond` outside a router state, several transitions for one event |
| 446 | B.2 | optional | `<datamodel>`, `cond` outside a router state, several transitions for one event |
| 448 | B.2 | optional | `<parallel>`, `<datamodel>`, `cond` outside a router state, several transitions for one event |
| 449 | B.2 | optional | `<datamodel>`, `cond` outside a router state, several transitions for one event |
| 451 | B.2 | optional | `<parallel>`, `<datamodel>`, several transitions for one event |
| 452 | B.2 | optional | `<raise>`, `<assign>`, `<script>`, `<datamodel>`, `cond` outside a router state, event wildcard `*` |
| 453 | B.2 | optional | `<raise>`, `<datamodel>`, `cond` outside a router state, event wildcard `*` |
| 456 | B.2 | optional | `<script>`, `<datamodel>`, several transitions for one event |
| 457 | B.2 | optional | `<raise>`, `<assign>`, `<foreach>`, `<log>`, `<datamodel>`, `cond` outside a router state, event wildcard `*`, several transitions for one event |
| 459 | B.2 | optional | `<assign>`, `<if>`, `<foreach>`, `<log>`, `<datamodel>`, `cond` outside a router state, several transitions for one event |
| 460 | B.2 | optional | `<assign>`, `<foreach>`, `<log>`, `<datamodel>`, `cond` outside a router state, several transitions for one event |
| 487 | 5.4 | mandatory | `<raise>`, `<assign>`, `<datamodel>`, event wildcard `*` |
| 488 | 5.7 | mandatory | `<donedata>`, `<param>`, `<datamodel>`, event wildcard `*`, several transitions for one event |
| 495 | C.1 | mandatory | `<send>`, event wildcard `*` |
| 496 | C.1 | mandatory | `<send>`, `<raise>`, `<datamodel>`, event wildcard `*` |
| 500 | C.1 | mandatory | `<datamodel>`, several transitions for one event |
| 501 | C.1 | mandatory | `<send>`, `<datamodel>`, event wildcard `*` |
| 503 | 3.13 | mandatory | `<raise>`, `<datamodel>`, several transitions for one event, targetless transition |
| 504 | 3.13 | mandatory | `<parallel>`, `<raise>`, `<datamodel>`, several transitions for one event |
| 505 | 3.13 | mandatory | `<raise>`, `<datamodel>`, several transitions for one event |
| 506 | 3.13 | mandatory | `<raise>`, `<datamodel>`, several transitions for one event |
| 509 | C.2 | optional | `<send>`, `<datamodel>`, event wildcard `*` |
| 510 | C.2 | optional | `<send>`, `<raise>`, `<datamodel>`, event wildcard `*` |
| 513 | C.2 | optional | manual |
| 518 | C.2 | optional | `<send>`, `<datamodel>`, event wildcard `*` |
| 519 | C.2 | optional | `<send>`, `<param>`, `<datamodel>`, event wildcard `*` |
| 520 | C.2 | optional | `<send>`, `<content>`, `<datamodel>`, event wildcard `*`, several transitions for one event |
| 521 | 6.2 | mandatory | `<send>`, `<datamodel>`, event wildcard `*` |
| 522 | C.2 | optional | `<send>`, `<datamodel>`, event wildcard `*` |
| 525 | 4.6 | mandatory | `<foreach>`, `<datamodel>`, several transitions for one event |
| 527 | 5.6 | mandatory | `<donedata>`, `<content>`, `<datamodel>`, several transitions for one event |
| 528 | 5.6 | mandatory | `<donedata>`, `<content>`, `<datamodel>`, event wildcard `*` |
| 529 | 5.6 | mandatory | `<donedata>`, `<content>`, `<datamodel>`, several transitions for one event |
| 530 | 6.4 | mandatory | `<invoke>` of an SCXML session, `<send>`, `<assign>`, `<content>`, `<datamodel>`, event wildcard `*` |
| 531 | C.2 | optional | `<send>`, `<param>`, `<datamodel>`, event wildcard `*` |
| 532 | C.2 | optional | `<send>`, `<content>`, `<datamodel>`, event wildcard `*` |
| 533 | 3.13 | mandatory | `<parallel>`, `<raise>`, `<datamodel>`, several transitions for one event |
| 534 | C.2 | optional | `<send>`, `<datamodel>`, event wildcard `*` |
| 550 | 5.3 | mandatory | `<datamodel>`, several transitions for one event |
| 551 | 5.3 | mandatory | `<datamodel>`, several transitions for one event |
| 552 | 5.3 | mandatory | `<datamodel>`, several transitions for one event |
| 553 | 6.2 | mandatory | `<send>`, `<datamodel>` |
| 554 | 6.4 | mandatory | `<invoke>` of an SCXML session, `<send>`, `<content>`, `<datamodel>` |
| 557 | B.2 | optional | `<datamodel>`, `cond` outside a router state, several transitions for one event |
| 558 | B.2 | optional | `<datamodel>`, `cond` outside a router state, several transitions for one event |
| 560 | B.2 | optional | `<send>`, `<param>`, `<datamodel>`, `cond` outside a router state, event wildcard `*` |
| 561 | B.2 | optional | `<send>`, `<content>`, `<datamodel>`, `cond` outside a router state, event wildcard `*` |
| 562 | B.2 | optional | `<send>`, `<content>`, `<datamodel>`, `cond` outside a router state, event wildcard `*` |
| 567 | C.2 | optional | `<send>`, `<assign>`, `<param>`, `<datamodel>`, event wildcard `*`, several transitions for one event |
| 569 | B.2 | optional | `<datamodel>`, `cond` outside a router state, several transitions for one event |
| 570 | 3.7 | mandatory | `<parallel>`, `<send>`, `<raise>`, `<assign>`, `<datamodel>`, event wildcard `*`, targetless transition |
| 576 | 3.2 | mandatory | `<parallel>`, `<send>`, `<raise>` |
| 577 | C.2 | optional | `<send>`, event wildcard `*` |
| 578 | B.2 | optional | `<send>`, `<content>`, `<datamodel>`, `cond` outside a router state, event wildcard `*` |
| 579 | 3.10 | mandatory | `<history>`, `<send>`, `<raise>`, `<datamodel>`, event wildcard `*`, `<initial>` element |
| 580 | 3.10 | mandatory | `<parallel>`, `<history>`, `<send>`, `<datamodel>`, several transitions for one event, `<initial>` element |
