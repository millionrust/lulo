"""AppleScript snippets shared by mac_inventory.py.

Every handler here is strictly read-only except where the task explicitly
calls for a reversible action: opening a menu with AXShowMenu then pressing
Escape, opening Settings with Cmd+, then closing it with Cmd+W, and
selecting a System Settings sidebar row to navigate (never toggling a
control). Nothing here clicks an item or flips a setting.
"""

# A submenu with more items than this is almost always a dynamically
# populated list — an open-window list, a shell-profile list, a font
# family list — rather than a static menu declaration, and walking it
# item-by-item (one AXMenuItemCmd* round trip per entry, recursively) is
# both slow and not useful: the diff does not try to match a live
# window/profile/font list entry-for-entry anyway. `dumpMenuItems` records
# such a submenu's item count instead of walking it. This is in addition
# to (not instead of) the always-skipped named submenus below, which are
# skipped regardless of size because they also leak the owner's own
# documents/account, not just because they are slow.
MAX_SUBMENU_WIDTH = 20

# Recursively dumps a menu's items as tab-separated lines:
#   depth \t title \t cmdChar \t cmdVirtualKey \t cmdModifiers \t enabled \t markChar \t omitReason
# `title` is empty for a separator. `omitReason` is non-empty exactly when
# this item has a submenu that was *not* walked (either because it is one
# of the always-skipped named submenus, or because it was wider than
# MAX_SUBMENU_WIDTH) — in which case no deeper-depth lines follow for it.
# Appended to every driver script.
DUMP_MENU_ITEMS_HANDLER = rf"""
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
			-- Services/Open Recent/Recent Items are populated system-wide
			-- (every installed app's Info.plist, or the owner's own
			-- documents) rather than declared by this app's menu, and can
			-- run to dozens of entries; recursing into them is both slow
			-- and liable to record the owner's personal document names.
			-- `mac_inventory.py` mirrors this list on the Python side
			-- (DYNAMIC_PERSONAL_SUBMENUS) as a defence-in-depth fallback
			-- for any raw dump that predates this check — keep both in
			-- sync.
			set subItemCount to 0
			set hasSub to false
			try
				set subMenus to menus of mi
				if (count of subMenus) > 0 then
					set hasSub to true
					set subItemCount to count of menu items of (item 1 of subMenus)
				end if
			end try
			set omitReason to ""
			if hasSub then
				if t is "Services" or t is "Open Recent" or t is "Recent Items" or t is "Apple" or t is "Import from iPhone" then
					set omitReason to "dynamic/personal submenu, not read"
				else if subItemCount > {MAX_SUBMENU_WIDTH} then
					set omitReason to "dynamic (" & subItemCount & " items), not read"
				end if
			end if
			set out to out & depth & tab & t & tab & cmdChar & tab & cmdVK & tab & cmdMods & tab & enState & tab & mk & tab & omitReason & linefeed
			if hasSub and omitReason is "" then
				try
					set out to out & my dumpMenuItems(item 1 of subMenus, depth + 1)
				end try
			end if
		end repeat
	end tell
	return out
end dumpMenuItems
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
