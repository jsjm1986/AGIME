from playwright.sync_api import sync_playwright
import json, time

with sync_playwright() as p:
    browser = p.chromium.launch(headless=True)
    page = browser.new_page()

    # Capture API responses
    captured = {}
    def handle_response(response):
        url = response.url
        if '/skills/available' in url or '/agents' in url:
            try:
                body = response.json()
                captured[url] = {'status': response.status, 'body': body}
            except:
                captured[url] = {'status': response.status, 'body': 'parse_error'}

    page.on('response', handle_response)

    # Navigate to team detail page
    page.goto('http://localhost:8080/teams/698616a1980c003c66f6421e')
    page.wait_for_load_state('networkidle')
    time.sleep(2)

    # Click on Agent tab
    agent_tab = page.locator('[role="tab"]').filter(has_text='Agent')
    if agent_tab.count() > 0:
        agent_tab.first.click()
        time.sleep(2)

    # Find agent cards and click edit
    page.screenshot(path='/tmp/step1_agent_tab.png', full_page=True)

    # Look for edit button - try various selectors
    all_buttons = page.locator('button').all()
    print(f"Total buttons on page: {len(all_buttons)}")
    for i, btn in enumerate(all_buttons[:20]):
        txt = btn.inner_text().strip()
        if txt:
            print(f"  btn[{i}]: '{txt}'")

    # Try clicking the pencil/edit icon on the first agent card
    # Look for the agent card area
    agent_cards = page.locator('.border.rounded-lg').all()
    print(f"\nAgent cards: {len(agent_cards)}")

    # Try to find edit button by looking for SVG icon buttons near agent name
    edit_btns = page.locator('button:has(svg)').all()
    print(f"SVG buttons: {len(edit_btns)}")

    # Click the first agent's edit button
    # The AgentManagePanel likely shows agents in a list with edit buttons
    # Let's try clicking on the agent name or an edit icon
    for btn in all_buttons:
        aria = btn.get_attribute('aria-label') or ''
        title = btn.get_attribute('title') or ''
        txt = btn.inner_text().strip()
        if any(kw in (aria + title + txt).lower() for kw in ['edit', '编辑', 'pencil']):
            print(f"Found edit button: text='{txt}', aria='{aria}', title='{title}'")
            btn.click()
            time.sleep(1)
            break
    else:
        # Try clicking on agent name to open edit
        agent_name_el = page.locator('text=agime').first
        if agent_name_el.count() > 0:
            # Find the parent card and look for buttons
            print("Trying to click near agent name...")
            # Click the first SVG button after the agent name
            for btn in edit_btns[:10]:
                try:
                    if btn.is_visible():
                        btn.click()
                        print(f"Clicked SVG button")
                        time.sleep(1)
                        break
                except:
                    continue

    page.screenshot(path='/tmp/step2_after_edit_click.png', full_page=True)

    # Check if dialog opened
    dialog = page.locator('[role="dialog"]')
    if dialog.count() > 0:
        print("\nDialog opened!")

        # Click on skills tab
        skills_tab = dialog.locator('[role="tab"]').filter(has_text='技能')
        if skills_tab.count() == 0:
            skills_tab = dialog.locator('[role="tab"]').filter(has_text='Skills')
        if skills_tab.count() == 0:
            skills_tab = dialog.locator('[role="tab"]').filter(has_text='skill')

        if skills_tab.count() > 0:
            skills_tab.first.click()
            time.sleep(1)
            page.screenshot(path='/tmp/step3_skills_tab.png', full_page=True)
            print("Skills tab clicked")

            # Get the skills tab content
            skills_content = dialog.inner_text()
            print(f"\nDialog text (first 500 chars):\n{skills_content[:500]}")

            # Click "添加技能" / "Add Skill" button
            add_btn = dialog.locator('button').filter(has_text='添加技能')
            if add_btn.count() == 0:
                add_btn = dialog.locator('button').filter(has_text='Add Skill')
            if add_btn.count() == 0:
                add_btn = dialog.locator('button').filter(has_text='添加')

            if add_btn.count() > 0:
                print(f"\nFound add skill button, clicking...")
                add_btn.first.click()
                time.sleep(2)  # Wait for API call
                page.screenshot(path='/tmp/step4_add_skill_dialog.png', full_page=True)

                # Check the add skill dialog content
                dialogs = page.locator('[role="dialog"]').all()
                print(f"Dialogs open: {len(dialogs)}")
                for i, d in enumerate(dialogs):
                    txt = d.inner_text()
                    print(f"\nDialog {i} text:\n{txt[:500]}")
            else:
                print("Add skill button not found")
                # List all buttons in dialog
                dialog_btns = dialog.locator('button').all()
                for btn in dialog_btns:
                    print(f"  Dialog button: '{btn.inner_text().strip()}'")
        else:
            print("Skills tab not found")
            # List all tabs
            tabs = dialog.locator('[role="tab"]').all()
            for tab in tabs:
                print(f"  Tab: '{tab.inner_text().strip()}'")
    else:
        print("Dialog not opened")
        # Check page content
        print(f"Page URL: {page.url}")
        print(f"Page title: {page.title()}")

    # Print captured API responses
    print("\n=== Captured API Responses ===")
    for url, data in captured.items():
        body_str = json.dumps(data['body'], ensure_ascii=False, indent=2)
        if len(body_str) > 500:
            body_str = body_str[:500] + '...'
        print(f"\n{url}")
        print(f"  Status: {data['status']}")
        print(f"  Body: {body_str}")

    browser.close()
