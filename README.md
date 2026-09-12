# niri-ds

A fork of [Niri](https://github.com/niri-wm/niri) with a small number of changes specifically aimed at improving functionality on **dual touchscreen setups**.

## What's different?

niri-ds currently consists primarily of **two upstream pull requests/commits** applied together to improve touchscreen functionality when using Niri with two physical displays that both have touch input:

* [PR #1856](https://github.com/niri-wm/niri/pull/1856)
* [PR #3984](https://github.com/niri-wm/niri/pull/3984)

These changes provide better touch input behavior across dual touchscreen setups while otherwise retaining the upstream Niri design and behavior.

Both changes are currently being worked toward upstream inclusion. This fork combines them so they can be used together in the meantime.

The goal of niri-ds is not to create a long-term alternative to Niri. It is simply a convenient way to test and use these changes on dual touchscreen devices before they are available in upstream Niri.

## Upstream

niri-ds is based on [niri-wm/niri](https://github.com/niri-wm/niri), the upstream Niri project.

For general Niri documentation, configuration, installation instructions, and information about the project, see the upstream repository.

## Status

niri-ds is an unofficial fork of Niri intended as a **temporary solution for dual touchscreen setups**.

The changes in this fork are intentionally small and focused. Once the changes provided by these PRs are implemented upstream, **this fork will become obsolete and will no longer be necessary**.

Until then, niri-ds provides a convenient way to use these changes together with Niri on dual touchscreen devices.

## License

niri-ds is licensed under the GNU General Public License, version 3 (GPL-3.0), the same license used by upstream Niri.

See the LICENSE file for the full license text.
