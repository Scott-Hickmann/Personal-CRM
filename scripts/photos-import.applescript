on run arguments
    if (count of arguments) is not 3 then error "invalid Photos import invocation"
    set imagePath to item 1 of arguments
    set albumName to item 2 of arguments
    set personID to item 3 of arguments
    set crmKeyword to "personal-crm:" & personID
    set imageFile to POSIX file imagePath

    tell application "Photos"
        launch
        set destinationAlbum to missing value
        repeat with candidateAlbum in albums
            if name of candidateAlbum is albumName then
                set destinationAlbum to candidateAlbum
                exit repeat
            end if
        end repeat
        if destinationAlbum is missing value then
            set destinationAlbum to make new album named albumName
        end if

        set importedItems to import {imageFile} into destinationAlbum skip check duplicates false
        if (count of importedItems) is 0 then
            error "Photos did not import the image; it may already exist in the library"
        end if
        set importedItem to item 1 of importedItems
        set existingKeywords to keywords of importedItem
        if existingKeywords is missing value then set existingKeywords to {}
        if existingKeywords does not contain "personal-crm" then set end of existingKeywords to "personal-crm"
        if existingKeywords does not contain crmKeyword then set end of existingKeywords to crmKeyword
        set keywords of importedItem to existingKeywords
        return id of importedItem
    end tell
end run
