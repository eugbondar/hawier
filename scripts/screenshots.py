# Capture console pages for the README (spec: build-tooling, "Console
# screenshots"). Runs inside the pinned Playwright image, started by
# scripts/screenshots.sh; it is not meant to be run on the host.
#
#   screenshots.py <console url> <out dir> <name> <path> [<name> <path> ...]
#
# Signs in through the login form, then visits each path and writes
# <out dir>/<name>.png: the window is WIDTH x HEIGHT, the picture is the
# whole page up to MAX_HEIGHT. Any page that answers non-2xx or shows a render
# error fails the run, naming the page: a broken page must never reach the
# README looking like a feature.
import sys

from playwright.sync_api import sync_playwright

WIDTH, HEIGHT = 1440, 900
MAX_HEIGHT = 1600
USER, PASSWORD, TENANT = "admin", "admin", "DEFAULT"
RENDER_ERROR = "template render failed"


def fail(message):
    print(f"screenshots: FAIL: {message}", file=sys.stderr)
    sys.exit(1)


def main():
    if len(sys.argv) < 5 or len(sys.argv) % 2 == 0:
        fail("usage: screenshots.py <console url> <out dir> <name> <path> [...]")
    base, out = sys.argv[1].rstrip("/"), sys.argv[2]
    pages = list(zip(sys.argv[3::2], sys.argv[4::2]))

    with sync_playwright() as p:
        # The engine may hand the container a proxy from its client config;
        # the stand is on loopback and must be reached directly.
        browser = p.chromium.launch(args=["--no-proxy-server"])
        context = browser.new_context(
            viewport={"width": WIDTH, "height": HEIGHT},
            device_scale_factor=1,
            color_scheme="dark",
        )
        page = context.new_page()

        page.goto(f"{base}/login")
        page.fill("input[name=username]", USER)
        page.fill("input[name=password]", PASSWORD)
        page.fill("input[name=tenant]", TENANT)
        with page.expect_navigation():
            page.locator("form").first.evaluate("f => f.requestSubmit()")
        if "/login" in page.url:
            fail(f"sign-in as {USER}@{TENANT} was refused")

        for name, path in pages:
            response = page.goto(f"{base}{path}", wait_until="networkidle")
            if response is None or not response.ok:
                status = response.status if response else "no response"
                fail(f"page '{name}' ({path}) answered {status}")
            if "/login" in page.url:
                fail(f"page '{name}' ({path}) sent the browser back to sign-in")
            if RENDER_ERROR in page.content():
                fail(f"page '{name}' ({path}) shows a render error")
            # The whole page, so sections below the fold (a target's action
            # history) are in the picture, cut at MAX_HEIGHT.
            height = min(page.evaluate("document.documentElement.scrollHeight"), MAX_HEIGHT)
            page.screenshot(
                path=f"{out}/{name}.png",
                full_page=True,
                clip={"x": 0, "y": 0, "width": WIDTH, "height": height},
            )
            print(f"screenshots: {name} <- {path}", flush=True)

        browser.close()


if __name__ == "__main__":
    main()
