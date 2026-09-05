on run argv
    if (count of argv) is not 3 then error "Expected contact ID, photo path, and backup path"
    set photoFile to POSIX file (item 2 of argv)
    set backupFile to POSIX file (item 3 of argv)

    tell application "Contacts"
        set matches to (every person whose id is item 1 of argv)
        if (count of matches) is not 1 then error "Contact disappeared or is ambiguous; refresh"
        set targetPerson to item 1 of matches
        if image of targetPerson is not missing value then error "Contact already has a photo; refusing to overwrite"
        set cardText to vcard of targetPerson
    end tell

    set backupHandle to open for access backupFile with write permission
    try
        set eof backupHandle to 0
        write cardText to backupHandle as «class utf8»
        close access backupHandle
    on error messageText number messageNumber
        try
            close access backupHandle
        end try
        error messageText number messageNumber
    end try

    tell application "Contacts"
        set image of targetPerson to (read photoFile as JPEG picture)
        save
    end tell
end run
