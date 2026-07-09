# niri-visual-tests

> [!NOTE]
>
> This is a development-only app, you shouldn't package it.

This app contains a number of hard-coded test scenarios for visual inspection.
It uses the real niri layout and rendering code, but with mock windows instead of Wayland clients.
The idea is to go through the test scenarios and check that everything *looks* right.

## Running

You will need recent GTK and libadwaita.
Then, `cargo run`.
