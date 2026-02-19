from playwright.sync_api import sync_playwright
import time

with sync_playwright() as p:
    browser = p.chromium.launch(headless=True)
    page = browser.new_page()

    # Navigate to the app
    page.goto('http://localhost:8080')
    page.wait_for_load_state('networkidle')

    # Check if we need to login
    if 'login' in page.url.lower() or page.locator('text=登录').count() > 0:
        print("Need to login first")
        page.screenshot(path='/tmp/login_page.png')

    # Navigate to team detail page
    page.goto('http://localhost:8080/teams/698616a1980c003c66f6421e')
    page.wait_for_load_state('networkidle')
    time.sleep(1)

    # Click on Agent tab
    agent_tab = page.locator('text=Agent').first
    if agent_tab.count() > 0:
        agent_tab.click()
        time.sleep(1)

    page.screenshot(path='/tmp/agent_tab.png')
    print(f"Current URL: {page.url}")

    # Find and click edit button on the first agent
    edit_btn = page.locator('button:has-text("编辑")').first
    if edit_btn.count() == 0:
        edit_btn = page.locator('[aria-label*="edit"], [aria-label*="编辑"]').first
    if edit_btn.count() == 0:
        # Try finding any edit icon button
        edit_btn = page.locator('button').filter(has=page.locator('svg')).nth(1)

    print(f"Edit buttons found: {page.locator('button:has-text(\"编辑\")').count()}")

    # Try clicking the edit/pencil icon on the agent card
    agent_cards = page.locator('[class*="border"][class*="rounded"]').all()
    print(f"Agent cards found: {len(agent_cards)}")

    # Take screenshot to see current state
    page.screenshot(path='/tmp/before_edit.png', full_page=True)

    # Look for any clickable element that opens edit dialog
    # The AgentManagePanel likely has edit buttons
    pencil_btns = page.locator('button').all()
    print(f"Total buttons: {len(pencil_btns)}")
    for i, btn in enumerate(pencil_btns[:15]):
        text = btn.inner_text().strip()
        if text:
            print(f"  Button {i}: '{text}'")

    # Try to find and click the edit button
    edit_found = False
    for btn in pencil_btns:
        text = btn.inner_text().strip()
        if '编辑' in text or 'Edit' in text:
            btn.click()
            edit_found = True
            print(f"Clicked edit button: '{text}'")
            break

    if not edit_found:
        # Try clicking on agent name/card to open edit
        agent_name = page.locator('text=agime').first
        if agent_name.count() > 0:
            # Look for edit button near the agent
            parent = agent_name.locator('..').locator('..')
            edit_in_card = parent.locator('button').all()
            print(f"Buttons near agent: {len(edit_in_card)}")
            for btn in edit_in_card:
                print(f"  Near-agent button: '{btn.inner_text().strip()}'")
                btn.click()
                edit_found = True
                break

    time.sleep(1)
    page.screenshot(path='/tmp/after_edit_click.png', full_page=True)

    # Check if dialog opened
    dialog = page.locator('[role="dialog"]')
    if dialog.count() > 0:
        print("Dialog opened!")

        # Click on 技能 tab
        skills_tab = page.locator('text=技能').first
        if skills_tab.count() > 0:
            skills_tab.click()
            time.sleep(0.5)
            page.screenshot(path='/tmp/skills_tab.png')
            print("Skills tab clicked")

            # Click 添加技能 button
            add_skill_btn = page.locator('text=添加技能').first
            if add_skill_btn.count() == 0:
                add_skill_btn = page.locator('text=Add Skill').first

            if add_skill_btn.count() > 0:
                add_skill_btn.click()
                time.sleep(2)  # Wait for API call
                page.screenshot(path='/tmp/add_skill_dialog.png')
                print("Add skill dialog opened")

                # Check what's in the dialog
                dialog_content = page.locator('[role="dialog"]').last.inner_text()
                print(f"Dialog content: {dialog_content[:500]}")
            else:
                print("Add skill button not found")
    else:
        print("Dialog not opened")
        # Check page content
        content = page.content()
        print(f"Page title: {page.title()}")

    browser.close()
