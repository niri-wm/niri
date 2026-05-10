niri's documentation files are found in `docs/wiki/` and should be viewable and browsable in at least three systems:

- The GitHub repo's markdown file preview
- [The GitHub repo's wiki](https://github.com/niri-wm/niri/wiki)
- [The documentation site](https://niri-wm.github.io/niri/)

## The GitHub repo's wiki

This is generated with the `publish-wiki` job in `.github/workflows/ci.yml`.
In order to have this job run as expected in your fork, you'll need to enable the wiki feature in your repo's settings on GitHub.
This could be useful as a contributor to verify that the wiki generates the way you expect it to.

## The documentation site

The documentation site is generated with [mkdocs](https://www.mkdocs.org/).
The configuration files are found in `docs/`.

To set up and run the documentation site locally, it is recommended to use [uv](https://docs.astral.sh/uv/).

### Serving the site locally with uv

In the `docs/` subdirectory:

- `uv sync`
- `uv run mkdocs serve`

The documentation site should now be available on http://127.0.0.1:8000/niri/

Changes made to the documentation while the development server is running will cause an automatic page refresh in the browser.

> [!TIP]
> Images may not be visible, as they are stored on Git LFS.
> If this is the case, run `git lfs pull`.

## Elements

Elements such as links, admonitions, images, and snippets should work as expected in markdown file previews on GitHub, the GitHub repo's wiki, and in the documentation site.

### Links

Links should in all cases be relative (e.g. `./FAQ.md`), unless it's an external one.
Links should have anchors if they are meant to lead the user to a specific section on a page (e.g. `./Getting-Started.md#nvidia`).

> [!TIP]
> mkdocs will terminate if relative links lead to non-existing documents or non-existing anchors.
> This means that the CI pipeline will fail when building documentation, as will `mkdocs serve` locally.

### Admonitions

> [!IMPORTANT]
> This is an important distinction from other `mkdocs`-based documentation you might have encountered.

Admonitions, or alerts should be written [the way GitHub defines them](https://docs.github.com/en/get-started/writing-on-github/getting-started-with-writing-and-formatting-on-github/basic-writing-and-formatting-syntax#alerts).
The above admonition is written like this:

```
> [!IMPORTANT]
> This is an important distinction from other `mkdocs`-based documentation you might have encountered.
```

### Images

Images should have relative links to resources in `docs/wiki/img/`, and should contain sensible alt-text.

### Videos

For compatibility with both mkdocs and GitHub Wiki, videos need to be wrapped in a `<video>` tag (displayed by mkdocs) and have the video link again as fallback text (displayed by GitHub Wiki) padded with blank lines.

```html
<video controls src="https://github.com/user-attachments/assets/379a5d1f-acdb-4c11-b36c-e85fd91f0995">

https://github.com/user-attachments/assets/379a5d1f-acdb-4c11-b36c-e85fd91f0995

</video>
```

### Snippets

Configuration and code snippets in general should be annotated with a language.

If the language used in the snippet is KDL, open the code block like this:

```md
```kdl
```
