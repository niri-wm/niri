# Contributing to niri

Thanks for your interest in niri!
The project has grown quite a bit, and we could use all help that we can.

Make sure to join our Matrix chat if you have any questions or want to discuss anything: https://matrix.to/#/#niri:matrix.org

## Issues and discussions

This is a good way to help many new and existing users without programming knowledge.

- Answer and help people in GitHub issues and discussions.
- Check and point out duplicate issues.
- Check for issues that are likely application bugs (and not niri bugs).
    - Ask or try to reproduce on another non-Smithay-based compositor (sway, KDE/KWin, GNOME/Mutter). If the issue reproduces, it's likely an application bug.
    - Ask or try to reproduce on another *Smithay-based* compositor ([cosmic-comp], [anvil]). If the issue reproduces only on Smithay compositors, it may be a Smithay bug.
    - Make sure you're testing the Wayland version of the app on all compositors. Apps may silently use X11 when an X11 `$DISPLAY` is available.
    - Problems with X11 apps should be reported to [xwayland-satellite]. When testing xwayland-satellite on different compositors, make sure you use xwayland-satellite's `$DISPLAY` (rather than another compositor's built-in Xwayland `$DISPLAY`).
    - After testing, mention where you could and couldn't reproduce, as well as the exact steps to reproduce if the issue is missing them.
- Try to reproduce the issue on your own system and write if you could or couldn't reproduce it.
- Upvote issues with a thumbs up reaction as you like.
- Ideas and feature requests from new users should go to Discussions.

If your issue is a duplicate, or not a niri issue (application bug, hardware problem, configuration problem), then please close it.

## Reviewing and testing pull requests

With the growing popularity, the volume of pull requests is honestly more than I can manage myself in my free time.
I would really appreciate help with testing and reviewing them.

### Testing

Pick a pull request you like, then build it and give it a go.
The [Developing niri wiki page](https://niri-wm.github.io/niri/Development:-Developing-niri) has guidance on running niri test builds.

Be really thorough with your testing.
We're striving for polished features in niri, so point out any issues and bugs, even small ones like animation jank.

- Think of weird edge cases or unexpected interactions and try them to see that they work reasonably.
- Try to break the feature and check that it behaves well.
- Where applicable, try different input devices: keyboard, mouse, trackpad, tablet, touchscreen.
- Watch out for any new performance drops.

For bug fixes, first make sure you can reproduce the bug, then do the same steps in the PR test build, and verify that the bug is fixed.
Be similarly thorough: test any similar or related edge cases to verify that the fix doesn't introduce any new problems.

Write your findings in the pull request: any issues you found, or if everything worked well.
Re-test after the author updates the code to see that your issues were fixed.

Don't hesitate to test even if someone else already did; very frequently different people will stumble upon different problems.

### Reviewing

Reviewing pull requests is something I need the most help with since there are a lot of them, and it's quite time-consuming.
Anyone with code accepted into niri is welcome, but this is not a requirement; even if you aren't familiar with Rust you may find some logic problems.

Pick a pull request, then review its code.

- Check that everything looks good, check various conditions for edge cases.
- See if there are any scenarios the author forgot to handle.
- Check that the code fits well into the rest of niri, follows its design and code style.
    - I understand this is vague. The idea is: look at the surrounding code and at similar modules (e.g. when implementing a new protocol, check other protocol implementations), and try to follow the style and structure.
- Check for unrelated changes that may be better split into their own pull request.
- Check that the wiki had been updated if necessary (for example, new config options were documented with examples, and have a correct Since annotation).

Point out everything you find as review comments (don't forget to submit the review).
Be constructive and respectful; some people may be new to programming and Rust.
As the author addresses the comments and issues, check the code again to see that the problems were fixed.
If everything looks good, say that, so I know someone has reviewed the PR.

As with testing, don't hesitate to look through and comment even if someone else already had.
Extra pairs of eyes catch more problems.

## Writing pull requests

When creating pull requests, please keep the following in mind.

- Make sure new features align with niri's design directions. Ideally, there should be an existing issue or discussion where we settled on that solution.
- Keep pull requests focused on a single feature or bug fix with no unrelated changes.
- Try to split your changes into small, self-contained commits. Every commit should build and pass tests. This makes it much easier to review your PR, and bisect for regressions in the future.
    - When addressing PR comments, try to squash the changes straight into the relevant commits.
    - In some cases when the requested changes are big/unclear, you can leave them as separate commits on top, but please squash and otherwise clean up the history when the changes are finalized.
    - To update the main branch, please rebase instead of merging. Try to force-push the main update rebase separately from other changes, this way it's easy to skip during review since it's usually not interesting.
    - When working on bigger features, I usually start with a big messy commit, then gradually split out smaller self-contained changes from it as the code gets into shape.
    - [git-rebase.io](https://git-rebase.io/) is a helpful guide for splitting commits and cleaning up history in git.
- When you address a review comment, mark it as resolved.
- Remember to [run tests](https://niri-wm.github.io/niri/Development:-Developing-niri#tests) and format the code with `cargo +nightly fmt --all`.
- For new layout actions, remember to add them to the randomized tests. For weird Wayland handling, adding client-server tests in `src/tests/` could be very useful.
- Test your changes by hand thoroughly, including for edge cases and weird interactions. See the Testing section above for some tips.
- Remember to document new config options on the wiki.
- When opening a pull request, ensure "Allow edits from maintainers" is enabled, so I can make final tweaks before merging.

### How to get your pull request reviewed more quickly

- Make it small and self-contained. Avoid mixing several unrelated changes in one PR.
- Split the PR into small and self-contained commits. This makes it much easier to review.
- Discuss new features, options, or behavior changes beforehand; make sure there's consensus about the design.
- When creating the pull request, clearly write what it does, what problem it solves, how to test it.
- Follow the rest of the advice from this document.

## AI contributions

If you use LLMs for your contribution (issue, comment, pull request), then it is *your job* to check and clean up its output, just like with any other tool.
*You* have to spend the time doing this.
Particularly:

- If I can tell that a pull request is mostly LLM-generated, then very likely this pull request will take *significantly more time and effort* than usual to review and finish. This is based on my prior review experience. Therefore, I'm not interested in such pull requests—there's always plenty of human-written ones which take priority.
- When using an LLM to prepare an issue, the text usually has a lot of unnecessary wording and irrelevant details. Anyone looking at such an issue will quickly lose interest in reading through it (myself certainly). Clean up the text and keep only those details that actually matter.
- When using an LLM to comment on an issue, *you* have to verify that the comment makes sense, contributes something useful, and doesn't have unnecessary repetition.


[cosmic-comp]: https://github.com/pop-os/cosmic-comp
[anvil]: https://github.com/Smithay/smithay/tree/master/anvil
[xwayland-satellite]: https://github.com/Supreeeme/xwayland-satellite
