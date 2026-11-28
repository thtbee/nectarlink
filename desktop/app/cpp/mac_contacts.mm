// SPDX-License-Identifier: GPL-3.0-or-later
// The Mac's own address book (Contacts.app), read through the Contacts framework.

#import <Contacts/Contacts.h>
#import <Foundation/Foundation.h>

#include <cstdlib>
#include <cstring>

extern "C" {

// Whether this app may read the Contacts: 1 yes, 0 refused, -1 can't ask (the app has no
// NSContactsUsageDescription, as when it runs from a terminal: asking then would end the
// process). Asks the first time, and waits for the answer.
int nl_mac_contacts_access(void)
{
    @autoreleasepool {
        CNAuthorizationStatus status = [CNContactStore authorizationStatusForEntityType:CNEntityTypeContacts];
        // 3 authorized; 4 limited (macOS 15), which still reads what was allowed.
        if (status == CNAuthorizationStatusAuthorized || (int)status == 4)
            return 1;
        if (status == CNAuthorizationStatusDenied || status == CNAuthorizationStatusRestricted)
            return 0;
        if (![[NSBundle mainBundle] objectForInfoDictionaryKey:@"NSContactsUsageDescription"])
            return -1;
        __block BOOL granted = NO;
        dispatch_semaphore_t done = dispatch_semaphore_create(0);
        [[CNContactStore new] requestAccessForEntityType:CNEntityTypeContacts
                                       completionHandler:^(BOOL ok, NSError *) {
                                           granted = ok;
                                           dispatch_semaphore_signal(done);
                                       }];
        dispatch_semaphore_wait(done, dispatch_time(DISPATCH_TIME_NOW, 120 * NSEC_PER_SEC));
        return granted ? 1 : 0;
    }
}

// The contacts that have a phone number, as JSON (malloc'd, free it with nl_mac_contacts_free):
// [{"id": text, "name": text, "numbers": [{"number": text, "label": text}], "photo": file name}].
// A contact's picture is written into `photo_dir` and named in "photo". NULL when it can't read.
char *nl_mac_contacts_fetch(const char *photo_dir)
{
    @autoreleasepool {
        NSString *dir = photo_dir ? [NSString stringWithUTF8String:photo_dir] : nil;
        NSArray *keys = @[
            CNContactIdentifierKey,
            [CNContactFormatter descriptorForRequiredKeysForStyle:CNContactFormatterStyleFullName],
            CNContactOrganizationNameKey,
            CNContactPhoneNumbersKey,
            CNContactThumbnailImageDataKey,
        ];
        CNContactFetchRequest *request = [[CNContactFetchRequest alloc] initWithKeysToFetch:keys];
        NSMutableArray *out = [NSMutableArray array];
        __block NSUInteger photos = 0;
        NSError *error = nil;
        BOOL ok = [[CNContactStore new]
            enumerateContactsWithFetchRequest:request
                                        error:&error
                                   usingBlock:^(CNContact *contact, BOOL *) {
                                       if (contact.phoneNumbers.count == 0)
                                           return;
                                       NSString *name = [CNContactFormatter stringFromContact:contact
                                                                                        style:CNContactFormatterStyleFullName];
                                       if (name.length == 0)
                                           name = contact.organizationName;
                                       if (name.length == 0)
                                           return;
                                       NSMutableArray *numbers = [NSMutableArray array];
                                       for (CNLabeledValue<CNPhoneNumber *> *entry in contact.phoneNumbers) {
                                           NSString *label = entry.label
                                               ? [CNLabeledValue localizedStringForLabel:entry.label]
                                               : @"";
                                           [numbers addObject:@{
                                               @"number" : entry.value.stringValue ?: @"",
                                               @"label" : label ?: @"",
                                           }];
                                       }
                                       NSMutableDictionary *item = [@{
                                           @"id" : contact.identifier,
                                           @"name" : name,
                                           @"numbers" : numbers,
                                       } mutableCopy];
                                       if (dir && contact.thumbnailImageData.length > 0) {
                                           NSString *file = [NSString stringWithFormat:@"p%lu", (unsigned long)photos++];
                                           if ([contact.thumbnailImageData
                                                   writeToFile:[dir stringByAppendingPathComponent:file]
                                                    atomically:NO])
                                               item[@"photo"] = file;
                                       }
                                       [out addObject:item];
                                   }];
        if (!ok)
            return (char *)NULL;
        NSData *json = [NSJSONSerialization dataWithJSONObject:out options:0 error:nil];
        if (!json)
            return (char *)NULL;
        char *copy = (char *)malloc(json.length + 1);
        if (!copy)
            return (char *)NULL;
        memcpy(copy, json.bytes, json.length);
        copy[json.length] = '\0';
        return copy;
    }
}

void nl_mac_contacts_free(char *json)
{
    free(json);
}

} // extern "C"
