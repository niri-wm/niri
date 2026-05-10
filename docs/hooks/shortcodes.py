# Copyright (c) 2016-2025 Martin Donath <martin.donath@squidfunk.com>

# Permission is hereby granted, free of charge, to any person obtaining a copy
# of this software and associated documentation files (the "Software"), to
# deal in the Software without restriction, including without limitation the
# rights to use, copy, modify, merge, publish, distribute, sublicense, and/or
# sell copies of the Software, and to permit persons to whom the Software is
# furnished to do so, subject to the following conditions:

# The above copyright notice and this permission notice shall be included in
# all copies or substantial portions of the Software.

# THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
# IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
# FITNESS FOR A PARTICULAR PURPOSE AND NON-INFRINGEMENT. IN NO EVENT SHALL THE
# AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
# LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
# FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS
# IN THE SOFTWARE.

from __future__ import annotations
import re
from re import Match

def on_page_markdown(
    markdown: str, *, page, config, files
):
    def replace(match: Match):
        matches = match.groups()
        preposition, version = matches[0], matches[1]
        return _badge_for_version(preposition, version)

    return re.sub(
        r"<sup>(Until|Since): (.*?)</sup>",
        replace, markdown, flags = re.I | re.M
    )

def _badge_for_version(preposition: str, version: str):
    if version == "next release":
        # we might fail to make real links to release notes on other cases too, but for now this is the one i've found
        return f"<span class=\"badge\">{preposition}: {version}</span>"
    else:
        path = f"https://github.com/niri-wm/niri/releases/tag/v{version}"
        return f"<span class=\"badge\">[{preposition}: {version}]({path})</span>"
