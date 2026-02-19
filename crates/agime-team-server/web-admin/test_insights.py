from playwright.sync_api import sync_playwright
import time

with sync_playwright() as p:
    browser = p.chromium.launch(headless=True)
    page = browser.new_page(viewport={"width": 1400, "height": 900})

    # Login first
    page.goto('http://localhost:8080')
    page.wait_for_load_state('networkidle')
    time.sleep(1)

    # Check if we need to login
    if page.locator('input[type="text"]').count() > 0:
        page.locator('input[type="text"]').fill('agime_30c73e_n21MGwWJViTOqaqQ1CNsXtJ4sc5ZBGXc')
        page.locator('button[type="submit"]').click()
        page.wait_for_load_state('networkidle')
        time.sleep(1)

    # Navigate to team detail page
    page.goto('http://localhost:8080/teams/698616a1980c003c66f6421e')
    page.wait_for_load_state('networkidle')
    time.sleep(2)

    # Click on 智能日志 tab
    smart_log_btn = page.locator('button:has-text("智能日志"), button:has-text("Smart Log")')
    if smart_log_btn.count() > 0:
        smart_log_btn.first.click()
        time.sleep(2)

    # Click AI 洞察 sub-tab
    insights_btn = page.locator('button:has-text("AI 洞察"), button:has-text("AI Insights")')
    if insights_btn.count() > 0:
        insights_btn.first.click()
        time.sleep(3)

    # Make sure 全部 filter is selected (click it)
    all_btn = page.locator('button:has-text("全部"), button:has-text("All")')
    if all_btn.count() > 0:
        all_btn.first.click()
        time.sleep(2)

    # Take screenshot
    page.screenshot(path='E:/yw/agiatme/goose/crates/agime-team-server/web-admin/insights_all_filter.png', full_page=True)

    # Also get the page content to check what's rendered
    # Count insight cards
    cards = page.locator('.rounded-lg.border-l-4.border-l-purple-500')
    print(f"Purple-bordered insight cards: {cards.count()}")

    # Count all section headers
    headers = page.locator('h3.text-lg.font-semibold')
    for i in range(headers.count()):
        print(f"Section header: {headers.nth(i).text_content()}")

    # Check for any Sparkles icons (insight items)
    sparkle_items = page.locator('.text-amber-500')
    print(f"Sparkle items (insight entries): {sparkle_items.count()}")

    # Get all visible text in the insights area
    main_content = page.locator('.space-y-6')
    if main_content.count() > 0:
        text = main_content.first.inner_text()
        print(f"\n--- Insights area text (first 2000 chars) ---")
        print(text[:2000])

    browser.close()
