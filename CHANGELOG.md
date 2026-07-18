# Changelog

## [0.4.0](https://github.com/khiet/grapevine/compare/grapevine-v0.3.0...grapevine-v0.4.0) (2026-07-18)


### Features

* increase popover height to 680px for better content visibility ([80bcb7f](https://github.com/khiet/grapevine/commit/80bcb7fe261665a2876146f92e569dc04c864634))
* list launch-at-login under Open at Login via SMAppService ([36577b0](https://github.com/khiet/grapevine/commit/36577b065aa27bfadcc800e89c006d6a9f30c121))
* list launch-at-login under Open at Login via SMAppService ([94786c3](https://github.com/khiet/grapevine/commit/94786c34de80b405a70b3164cad6818311edbe19))

## [0.3.0](https://github.com/khiet/grapevine/compare/grapevine-v0.2.0...grapevine-v0.3.0) (2026-07-18)


### Features

* **github,ui:** blocked-PR dot and draft pill on the PR row ([9d15892](https://github.com/khiet/grapevine/commit/9d158925efde6910378e08163df7168229f8f990))
* **github:** CI status indicator on the PR row ([4d3146a](https://github.com/khiet/grapevine/commit/4d3146a9decb8659f5bbaf28b8e20a162cae497c))
* **github:** show organization avatar badge on each PR row ([388a018](https://github.com/khiet/grapevine/commit/388a018d0b5b7fe3a0671c5a7b5aa65e64cd9de8))
* **ui:** point the blocked-dot tooltip at the dot from its left ([d2f181e](https://github.com/khiet/grapevine/commit/d2f181e03a051f925aaaacfad9819d2664955e0d))
* **ui:** show the blocked-dot tooltip after 200ms instead of the native delay ([c0b6d31](https://github.com/khiet/grapevine/commit/c0b6d317975110c177ba57db6e9aaf8375cdd332))


### Bug Fixes

* **github:** order PRs by updated_at instead of created_at ([23e2f3d](https://github.com/khiet/grapevine/commit/23e2f3daf61be5f0080989a8119ad6ab433728c0))

## [0.2.0](https://github.com/khiet/grapevine/compare/grapevine-v0.1.0...grapevine-v0.2.0) (2026-07-17)


### Features

* group the All section into a slab per repo ([7f35a86](https://github.com/khiet/grapevine/commit/7f35a86ba3ce5d4d8db4daa2e1866792f8ef1746))
* live PR list synced from GitHub GraphQL ([6d565ce](https://github.com/khiet/grapevine/commit/6d565ce711ba4506f8443e49606ae1274441a723))
* merged section with manual dismiss ([d951d02](https://github.com/khiet/grapevine/commit/d951d0275e56a6d2483ed9cfde940562777277d0))
* merged section with manual dismiss and clear-all ([e4ced55](https://github.com/khiet/grapevine/commit/e4ced55c11d8f48a9229152db8f80702a0746f00))
* polish the PR row with avatar, username, updated timestamp, and red unread badge ([cf18bc3](https://github.com/khiet/grapevine/commit/cf18bc3031c0316ea1553bebf962875c4834c326))
* prefilled Create Token link and token scope verification ([9e5ce58](https://github.com/khiet/grapevine/commit/9e5ce58770242d49005fb2f09666867b94967daa))
* replace stock icons with the Trellis grape-merge design ([b7d8679](https://github.com/khiet/grapevine/commit/b7d86795be162fa7037031cc80cba7974f37fd33))
* replace stock icons with the Trellis grape-merge design ([62de1a0](https://github.com/khiet/grapevine/commit/62de1a065b0003f6ce2a4973324ae29e70f7ba5a))
* restyle the settings screen as card slabs matching the PR list ([c9fb968](https://github.com/khiet/grapevine/commit/c9fb9683ffbef1d5d2e2f0b21124e77c01d91a3a))
* scaffold Tauri v2 menubar app with tray-toggled React popover ([9d57412](https://github.com/khiet/grapevine/commit/9d574129921f3a592a9a80c28b238768a71a0fea))
* settings view with Keychain-stored PAT and validated repo watchlist ([fba4d36](https://github.com/khiet/grapevine/commit/fba4d36f73482c12582249c35a6c12a06b9767f5))
* sync robustness, configurable polling, and shell polish ([26db26b](https://github.com/khiet/grapevine/commit/26db26bb5d4f7bffb34a0eb79ec95b11776a506e))
* sync robustness, configurable polling, and shell polish PAVE-3612 ([d1cd625](https://github.com/khiet/grapevine/commit/d1cd625d469169fe89b312c18e44cb1f21faca0c))
* unread engine with per-PR badges, tray count, and mark-as-read ([e02a6de](https://github.com/khiet/grapevine/commit/e02a6defde6f144c6085d20df49ad144b7a7f827))
* unread engine with per-PR badges, tray count, and mark-as-read ([85c551d](https://github.com/khiet/grapevine/commit/85c551d6deb50334576df67cbb1d47a16309992b))


### Bug Fixes

* bold scope names in the token note and drop em dashes from copy ([afd90b1](https://github.com/khiet/grapevine/commit/afd90b1167bd6bad89a9c25060b4896085f6115f))
* center the avatar and unread badge against the PR row text ([ad602df](https://github.com/khiet/grapevine/commit/ad602df001492b7103032c1ebcaf41056f5ff8d2))
* clear the tray count with an empty title instead of None ([fc70d54](https://github.com/khiet/grapevine/commit/fc70d54dc9665d664eccc53a4933dfe18b530732))
* hover merged rows with the accent tint, not a grey darken ([3a8add6](https://github.com/khiet/grapevine/commit/3a8add6f97acb2a17f03b705894067159fa22021))
* render 52-week-old PR ages as 1y instead of 0y ([f48be4f](https://github.com/khiet/grapevine/commit/f48be4f5b024e640301bd987ba35292738257a71))
* tint the PR row hover with the accent instead of grey ([8b25dbf](https://github.com/khiet/grapevine/commit/8b25dbfe58f6edc2381fb5bd05e0e85328629332))
