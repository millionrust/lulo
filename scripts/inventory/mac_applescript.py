"""AppleScript snippets shared by mac_inventory.py.

Every handler here is strictly read-only except where the task explicitly
calls for a reversible action: opening a menu with AXShowMenu then pressing
Escape, opening Settings with Cmd+, then closing it with Cmd+W, and
selecting a System Settings sidebar row to navigate (never toggling a
control). Nothing here clicks an item or flips a setting.
"""

# Recursively dumps a menu's items as tab-separated lines:
#   depth \t title \t cmdChar \t cmdVirtualKey \t cmdModifiers \t enabled \t markChar
# `title` is empty for a separator. Appended to every driver script.
DUMP_MENU_ITEMS_HANDLER = r"""
on dumpMenuItems(menuRef, depth)
	set out to ""
	tell application "System Events"
		set mis to menu items of menuRef
		repeat with mi in mis
			set t to ""
			try
				set nm to name of mi
				if nm is not missing value then set t to nm
			end try
			set cmdChar to ""
			try
				set cc to value of attribute "AXMenuItemCmdChar" of mi
				if cc is not missing value then set cmdChar to cc
			end try
			set cmdVK to ""
			try
				set vk to value of attribute "AXMenuItemCmdVirtualKey" of mi
				if vk is not missing value then set cmdVK to (vk as string)
			end try
			set cmdMods to ""
			try
				set cm to value of attribute "AXMenuItemCmdModifiers" of mi
				if cm is not missing value then set cmdMods to (cm as string)
			end try
			set enState to "1"
			try
				if not (enabled of mi) then set enState to "0"
			end try
			set mk to ""
			try
				set mkv to value of attribute "AXMenuItemMarkChar" of mi
				if mkv is not missing value then set mk to mkv
			end try
			set subCount to 0
			try
				set subCount to count of menus of mi
			end try
			set out to out & depth & tab & t & tab & cmdChar & tab & cmdVK & tab & cmdMods & tab & enState & tab & mk & linefeed
			-- Services/Open Recent/Recent Items are populated system-wide
			-- (every installed app's Info.plist, or the owner's own
			-- documents) rather than declared by this app's menu, and can
			-- run to dozens of entries; recursing into them is both slow
			-- and liable to record the owner's personal document names.
			-- `mac_inventory.py` already drops their children on the
			-- Python side (DYNAMIC_PERSONAL_SUBMENUS) — skip the walk here
			-- too so a Services-heavy app does not time the whole run out.
			if subCount > 0 and t is not "Services" and t is not "Open Recent" and t is not "Recent Items" and t is not "Apple" then
				try
					set out to out & my dumpMenuItems(menu 1 of mi, depth + 1)
				end try
			end if
		end repeat
	end tell
	return out
end dumpMenuItems
"""

DUMP_MENU_BAR_HANDLER = r"""
on dumpMenuBar(procName)
	set out to ""
	tell application "System Events"
		tell process procName
			set topItems to menu bar items of menu bar 1
			repeat with topItem in topItems
				set topName to ""
				try
					set topName to name of topItem
				end try
				set out to out & "0" & tab & topName & tab & tab & tab & tab & "1" & tab & linefeed
				try
					set out to out & my dumpMenuItems(menu 1 of topItem, 1)
				end try
			end repeat
		end tell
	end tell
	return out
end dumpMenuBar
"""

# Depth-first search for the first AXOutline or AXTable under `elementRef`
# (a sidebar list, wherever it is nested inside split groups/scroll areas).
# Returns `missing value` if none is found within `maxDepth`.
FIND_LIST_HANDLER = r"""
on findList(elementRef, depth, maxDepth)
	if depth > maxDepth then return missing value
	tell application "System Events"
		try
			set r to role of elementRef
			if r is "AXOutline" or r is "AXTable" then return elementRef
		end try
		try
			set kids to UI elements of elementRef
		on error
			return missing value
		end try
		repeat with kid in kids
			set found to my findList(kid, depth + 1, maxDepth)
			if found is not missing value then return found
		end repeat
	end tell
	return missing value
end findList
"""

# Dumps every control's role and title/value under `elementRef`, bounded to
# `maxDepth`, as tab-separated lines: depth \t role \t title.
DUMP_CONTROLS_HANDLER = r"""
on dumpControls(elementRef, depth, maxDepth)
	set out to ""
	if depth > maxDepth then return out
	tell application "System Events"
		try
			set kids to UI elements of elementRef
		on error
			return out
		end try
		repeat with kid in kids
			set r to ""
			try
				set r to role of kid
			end try
			set t to ""
			try
				set nm to title of kid
				if nm is not missing value and nm is not "" then set t to nm
			end try
			if t is "" then
				try
					set nm to value of kid
					if class of nm is text and nm is not "" then set t to nm
				end try
			end if
			if t is "" then
				try
					set nm to description of kid
					if nm is not missing value and nm is not "" then set t to nm
				end try
			end if
			if r is not "" then
				set out to out & depth & tab & r & tab & t & linefeed
			end if
			set out to out & my dumpControls(kid, depth + 1, maxDepth)
		end repeat
	end tell
	return out
end dumpControls
"""


def driver(*handlers: str, body: str) -> str:
	return "\n".join(handlers) + "\n" + body
