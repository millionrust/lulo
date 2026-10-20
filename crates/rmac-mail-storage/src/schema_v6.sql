-- MROBOT-03: View ▸ Sort By ▸ Size and Show Message Size. The message's
-- RFC822.SIZE in bytes as the server reports it (or its downloaded source's
-- length); 0 until known. Older rows learn theirs when their body arrives.
ALTER TABLE messages ADD COLUMN size INTEGER NOT NULL DEFAULT 0;
