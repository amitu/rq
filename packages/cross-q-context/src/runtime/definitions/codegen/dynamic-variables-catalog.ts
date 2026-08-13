// cross-q-context — vendored dynamic-variable METADATA catalog (self-contained, zero app dep).
//
// The editor-type codegen (generate-types.ts) emits `rq.dynamic.d.ts` — the typed `$guid`/
// `$randomInt`/faker.* signatures the in-editor autocomplete shows. It needs only the dynamic
// variables' METADATA (name/label/description/args/example), NOT their faker value-generation
// (which the HOST injects at runtime via rq.* — cross-q-context never generates values).
//
// This is a generated SNAPSHOT of the app's DynamicVariableResolver.list() (123 entries). It is
// data, not logic: regenerate when the dynamic-variable catalog changes. A parity check
// (app resolver.list() ⊆ this snapshot) guards drift — these are editor hints only, so a stale
// entry degrades autocomplete, never runtime correctness.

/** One argument of a dynamic variable (e.g. `min`/`max` of `$randomInt`). */
export interface VariableArgMetadata {
  name: string;
  description: string;
  type: string;
  optional?: boolean;
  defaultValue?: string;
  example?: string;
  order: number;
}

/** Editor-facing metadata for one dynamic variable. */
export interface VariableMetadata {
  name: string;
  label: string;
  description: string;
  example: string;
  category: string;
  args?: readonly VariableArgMetadata[];
}

/** The vendored dynamic-variable catalog — snapshot of the app's DynamicVariableResolver.list(). */
export const DYNAMIC_VARIABLE_CATALOG: readonly VariableMetadata[] = [
  {
    "name": "$guid",
    "label": "GUID",
    "description": "UUID v4",
    "example": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
    "category": "Common"
  },
  {
    "name": "$randomUUID",
    "label": "Random UUID",
    "description": "UUID v4 (alias for $guid)",
    "example": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
    "category": "Common"
  },
  {
    "name": "$timestamp",
    "label": "Timestamp",
    "description": "Current Unix timestamp in seconds",
    "example": "1711276800",
    "category": "Common"
  },
  {
    "name": "$isoTimestamp",
    "label": "ISO Timestamp",
    "description": "Current ISO 8601 datetime string",
    "example": "2024-03-24T12:00:00.000Z",
    "category": "Common"
  },
  {
    "name": "$randomInt",
    "label": "Random Integer",
    "description": "Random integer between min and max (inclusive)",
    "example": "42",
    "category": "Common",
    "args": [
      {
        "name": "min",
        "description": "Minimum value (inclusive)",
        "type": "number",
        "optional": true,
        "defaultValue": "0",
        "example": "1",
        "order": 0
      },
      {
        "name": "max",
        "description": "Maximum value (inclusive)",
        "type": "number",
        "optional": true,
        "defaultValue": "1000",
        "example": "100",
        "order": 1
      }
    ]
  },
  {
    "name": "$randomFirstName",
    "label": "Random First Name",
    "description": "Random first name",
    "example": "John",
    "category": "Person",
    "args": [
      {
        "name": "sex",
        "description": "Gender: \"male\" or \"female\"",
        "type": "string",
        "optional": true,
        "example": "female",
        "order": 0
      }
    ]
  },
  {
    "name": "$randomLastName",
    "label": "Random Last Name",
    "description": "Random last name",
    "example": "Doe",
    "category": "Person",
    "args": [
      {
        "name": "sex",
        "description": "Gender: \"male\" or \"female\"",
        "type": "string",
        "optional": true,
        "example": "male",
        "order": 0
      }
    ]
  },
  {
    "name": "$randomFullName",
    "label": "Random Full Name",
    "description": "Random full name",
    "example": "John Doe",
    "category": "Person",
    "args": [
      {
        "name": "sex",
        "description": "Gender: \"male\" or \"female\"",
        "type": "string",
        "optional": true,
        "example": "female",
        "order": 0
      }
    ]
  },
  {
    "name": "$randomNamePrefix",
    "label": "Random Name Prefix",
    "description": "Random name prefix",
    "example": "Mr.",
    "category": "Person"
  },
  {
    "name": "$randomNameSuffix",
    "label": "Random Name Suffix",
    "description": "Random name suffix",
    "example": "Jr.",
    "category": "Person"
  },
  {
    "name": "$randomJobTitle",
    "label": "Random Job Title",
    "description": "Random job title",
    "example": "Software Engineer",
    "category": "Person"
  },
  {
    "name": "$randomJobArea",
    "label": "Random Job Area",
    "description": "Random job area",
    "example": "Functionality",
    "category": "Person"
  },
  {
    "name": "$randomJobDescriptor",
    "label": "Random Job Descriptor",
    "description": "Random job descriptor",
    "example": "Senior",
    "category": "Person"
  },
  {
    "name": "$randomJobType",
    "label": "Random Job Type",
    "description": "Random job type",
    "example": "Designer",
    "category": "Person"
  },
  {
    "name": "$randomEmail",
    "label": "Random Email",
    "description": "Random email address",
    "example": "john.doe@example.com",
    "category": "Internet",
    "args": [
      {
        "name": "firstName",
        "description": "First name to use",
        "type": "string",
        "optional": true,
        "example": "John",
        "order": 0
      },
      {
        "name": "lastName",
        "description": "Last name to use",
        "type": "string",
        "optional": true,
        "example": "Doe",
        "order": 1
      },
      {
        "name": "provider",
        "description": "Email provider domain",
        "type": "string",
        "optional": true,
        "example": "example.com",
        "order": 2
      },
      {
        "name": "allowSpecialCharacters",
        "description": "Allow special characters",
        "type": "boolean",
        "optional": true,
        "defaultValue": "false",
        "example": "true",
        "order": 3
      }
    ]
  },
  {
    "name": "$randomUserName",
    "label": "Random Username",
    "description": "Random username",
    "example": "johndoe42",
    "category": "Internet",
    "args": [
      {
        "name": "firstName",
        "description": "First name to use",
        "type": "string",
        "optional": true,
        "example": "John",
        "order": 0
      },
      {
        "name": "lastName",
        "description": "Last name to use",
        "type": "string",
        "optional": true,
        "example": "Doe",
        "order": 1
      }
    ]
  },
  {
    "name": "$randomUrl",
    "label": "Random URL",
    "description": "Random URL",
    "example": "https://example.com",
    "category": "Internet",
    "args": [
      {
        "name": "protocol",
        "description": "URL protocol",
        "type": "string",
        "optional": true,
        "defaultValue": "https",
        "example": "http",
        "order": 0
      },
      {
        "name": "appendSlash",
        "description": "Append trailing slash",
        "type": "boolean",
        "optional": true,
        "defaultValue": "false",
        "example": "true",
        "order": 1
      }
    ]
  },
  {
    "name": "$randomIP",
    "label": "Random IP",
    "description": "Random IPv4 address",
    "example": "192.168.1.1",
    "category": "Internet"
  },
  {
    "name": "$randomIPV6",
    "label": "Random IPv6",
    "description": "Random IPv6 address",
    "example": "2001:0db8:85a3:0000:0000:8a2e:0370:7334",
    "category": "Internet"
  },
  {
    "name": "$randomMACAddress",
    "label": "Random MAC Address",
    "description": "Random MAC address",
    "example": "00:1B:44:11:3A:B7",
    "category": "Internet",
    "args": [
      {
        "name": "separator",
        "description": "Separator character",
        "type": "string",
        "optional": true,
        "defaultValue": ":",
        "example": "-",
        "order": 0
      }
    ]
  },
  {
    "name": "$randomPassword",
    "label": "Random Password",
    "description": "Random password string",
    "example": "xK9#mP2$vL",
    "category": "Internet",
    "args": [
      {
        "name": "length",
        "description": "Password length",
        "type": "number",
        "optional": true,
        "defaultValue": "16",
        "example": "32",
        "order": 0
      },
      {
        "name": "memorable",
        "description": "Generate memorable password",
        "type": "boolean",
        "optional": true,
        "defaultValue": "false",
        "example": "true",
        "order": 1
      },
      {
        "name": "prefix",
        "description": "Prefix string",
        "type": "string",
        "optional": true,
        "example": "APP_",
        "order": 2
      }
    ]
  },
  {
    "name": "$randomUserAgent",
    "label": "Random User Agent",
    "description": "Random browser user agent string",
    "example": "Mozilla/5.0 ...",
    "category": "Internet"
  },
  {
    "name": "$randomDomainName",
    "label": "Random Domain Name",
    "description": "Random domain name",
    "example": "example.com",
    "category": "Internet"
  },
  {
    "name": "$randomDomainSuffix",
    "label": "Random Domain Suffix",
    "description": "Random domain suffix",
    "example": ".com",
    "category": "Internet"
  },
  {
    "name": "$randomDomainWord",
    "label": "Random Domain Word",
    "description": "Random domain word",
    "example": "example",
    "category": "Internet"
  },
  {
    "name": "$randomLocale",
    "label": "Random Locale",
    "description": "Random two-letter language code (ISO 639-1)",
    "example": "fr",
    "category": "Internet"
  },
  {
    "name": "$randomProtocol",
    "label": "Random Protocol",
    "description": "Random internet protocol (http or https)",
    "example": "https",
    "category": "Internet"
  },
  {
    "name": "$randomExampleEmail",
    "label": "Random Example Email",
    "description": "Random email using example domain",
    "example": "john.doe@example.com",
    "category": "Internet",
    "args": [
      {
        "name": "firstName",
        "description": "First name to use",
        "type": "string",
        "optional": true,
        "example": "John",
        "order": 0
      },
      {
        "name": "lastName",
        "description": "Last name to use",
        "type": "string",
        "optional": true,
        "example": "Doe",
        "order": 1
      }
    ]
  },
  {
    "name": "$randomCity",
    "label": "Random City",
    "description": "Random city name",
    "example": "San Francisco",
    "category": "Location"
  },
  {
    "name": "$randomStreetName",
    "label": "Random Street Name",
    "description": "Random street name",
    "example": "Main St",
    "category": "Location"
  },
  {
    "name": "$randomStreetAddress",
    "label": "Random Street Address",
    "description": "Random street address",
    "example": "123 Main St",
    "category": "Location",
    "args": [
      {
        "name": "useFullAddress",
        "description": "Include secondary address",
        "type": "boolean",
        "optional": true,
        "defaultValue": "false",
        "example": "true",
        "order": 0
      }
    ]
  },
  {
    "name": "$randomCountry",
    "label": "Random Country",
    "description": "Random country name",
    "example": "United States",
    "category": "Location"
  },
  {
    "name": "$randomCountryCode",
    "label": "Random Country Code",
    "description": "Random ISO country code",
    "example": "US",
    "category": "Location",
    "args": [
      {
        "name": "variant",
        "description": "\"alpha-2\" or \"alpha-3\"",
        "type": "string",
        "optional": true,
        "defaultValue": "alpha-2",
        "example": "alpha-3",
        "order": 0
      }
    ]
  },
  {
    "name": "$randomLatitude",
    "label": "Random Latitude",
    "description": "Random latitude coordinate",
    "example": "37.7749",
    "category": "Location",
    "args": [
      {
        "name": "min",
        "description": "Minimum latitude",
        "type": "number",
        "optional": true,
        "defaultValue": "-90",
        "example": "30",
        "order": 0
      },
      {
        "name": "max",
        "description": "Maximum latitude",
        "type": "number",
        "optional": true,
        "defaultValue": "90",
        "example": "50",
        "order": 1
      },
      {
        "name": "precision",
        "description": "Decimal precision",
        "type": "number",
        "optional": true,
        "defaultValue": "4",
        "example": "6",
        "order": 2
      }
    ]
  },
  {
    "name": "$randomLongitude",
    "label": "Random Longitude",
    "description": "Random longitude coordinate",
    "example": "-122.4194",
    "category": "Location",
    "args": [
      {
        "name": "min",
        "description": "Minimum longitude",
        "type": "number",
        "optional": true,
        "defaultValue": "-180",
        "example": "-120",
        "order": 0
      },
      {
        "name": "max",
        "description": "Maximum longitude",
        "type": "number",
        "optional": true,
        "defaultValue": "180",
        "example": "-80",
        "order": 1
      },
      {
        "name": "precision",
        "description": "Decimal precision",
        "type": "number",
        "optional": true,
        "defaultValue": "4",
        "example": "6",
        "order": 2
      }
    ]
  },
  {
    "name": "$randomZipCode",
    "label": "Random Zip Code",
    "description": "Random zip/postal code",
    "example": "94105",
    "category": "Location"
  },
  {
    "name": "$randomPhoneNumber",
    "label": "Random Phone Number",
    "description": "Random phone number",
    "example": "+1-555-0123",
    "category": "Location",
    "args": [
      {
        "name": "style",
        "description": "\"human\", \"national\", or \"international\"",
        "type": "string",
        "optional": true,
        "example": "international",
        "order": 0
      }
    ]
  },
  {
    "name": "$randomPhoneNumberExt",
    "label": "Random Phone Number with Extension",
    "description": "Random phone number with extension",
    "example": "+1-555-0123 x456",
    "category": "Location"
  },
  {
    "name": "$randomBankAccount",
    "label": "Random Bank Account",
    "description": "Random bank account number",
    "example": "12345678",
    "category": "Finance",
    "args": [
      {
        "name": "length",
        "description": "Account number length",
        "type": "number",
        "optional": true,
        "defaultValue": "8",
        "example": "10",
        "order": 0
      }
    ]
  },
  {
    "name": "$randomBankAccountName",
    "label": "Random Bank Account Name",
    "description": "Random bank account name",
    "example": "Savings Account",
    "category": "Finance"
  },
  {
    "name": "$randomCreditCardNumber",
    "label": "Random Credit Card",
    "description": "Random credit card number",
    "example": "4111111111111111",
    "category": "Finance"
  },
  {
    "name": "$randomBankAccountIban",
    "label": "Random IBAN",
    "description": "Random IBAN number",
    "example": "DE89370400440532013000",
    "category": "Finance",
    "args": [
      {
        "name": "formatted",
        "description": "Format with spaces",
        "type": "boolean",
        "optional": true,
        "defaultValue": "false",
        "example": "true",
        "order": 0
      },
      {
        "name": "countryCode",
        "description": "ISO country code",
        "type": "string",
        "optional": true,
        "example": "DE",
        "order": 1
      }
    ]
  },
  {
    "name": "$randomBankAccountBic",
    "label": "Random BIC",
    "description": "Random BIC/SWIFT code",
    "example": "DEUTDEFF",
    "category": "Finance",
    "args": [
      {
        "name": "includeBranchCode",
        "description": "Include branch code",
        "type": "boolean",
        "optional": true,
        "defaultValue": "false",
        "example": "true",
        "order": 0
      }
    ]
  },
  {
    "name": "$randomTransactionType",
    "label": "Random Transaction Type",
    "description": "Random transaction type",
    "example": "payment",
    "category": "Finance"
  },
  {
    "name": "$randomCurrencyCode",
    "label": "Random Currency Code",
    "description": "Random ISO 4217 currency code",
    "example": "USD",
    "category": "Finance"
  },
  {
    "name": "$randomCurrencyName",
    "label": "Random Currency Name",
    "description": "Random currency name",
    "example": "US Dollar",
    "category": "Finance"
  },
  {
    "name": "$randomCurrencySymbol",
    "label": "Random Currency Symbol",
    "description": "Random currency symbol",
    "example": "$",
    "category": "Finance"
  },
  {
    "name": "$randomBitcoinAddress",
    "label": "Random Bitcoin Address",
    "description": "Random Bitcoin address",
    "example": "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa",
    "category": "Finance"
  },
  {
    "name": "$randomBitcoin",
    "label": "Random Bitcoin Address (Alias)",
    "description": "Random Bitcoin address (alias for $randomBitcoinAddress)",
    "example": "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa",
    "category": "Finance"
  },
  {
    "name": "$randomCreditCardMask",
    "label": "Random Credit Card Mask",
    "description": "Last 4 digits of a random credit card number",
    "example": "1234",
    "category": "Finance"
  },
  {
    "name": "$randomAlphaNumeric",
    "label": "Random Alphanumeric",
    "description": "Random alphanumeric string",
    "example": "a7b3",
    "category": "Text",
    "args": [
      {
        "name": "length",
        "description": "String length",
        "type": "number",
        "optional": true,
        "defaultValue": "1",
        "example": "8",
        "order": 0
      },
      {
        "name": "casing",
        "description": "\"upper\", \"lower\", or \"mixed\"",
        "type": "string",
        "optional": true,
        "defaultValue": "mixed",
        "example": "lower",
        "order": 1
      }
    ]
  },
  {
    "name": "$randomLoremWord",
    "label": "Random Word",
    "description": "Random lorem ipsum word",
    "example": "lorem",
    "category": "Text",
    "args": [
      {
        "name": "length",
        "description": "Word length",
        "type": "number",
        "optional": true,
        "example": "8",
        "order": 0
      }
    ]
  },
  {
    "name": "$randomLoremSentence",
    "label": "Random Sentence",
    "description": "Random lorem ipsum sentence",
    "example": "Lorem ipsum dolor sit amet.",
    "category": "Text",
    "args": [
      {
        "name": "wordCount",
        "description": "Number of words",
        "type": "number",
        "optional": true,
        "example": "5",
        "order": 0
      }
    ]
  },
  {
    "name": "$randomLoremParagraph",
    "label": "Random Paragraph",
    "description": "Random lorem ipsum paragraph",
    "example": "Lorem ipsum dolor sit amet, consectetur...",
    "category": "Text",
    "args": [
      {
        "name": "sentenceCount",
        "description": "Number of sentences",
        "type": "number",
        "optional": true,
        "example": "3",
        "order": 0
      }
    ]
  },
  {
    "name": "$randomLoremSlug",
    "label": "Random Slug",
    "description": "Random URL-friendly slug",
    "example": "lorem-ipsum-dolor",
    "category": "Text"
  },
  {
    "name": "$randomWords",
    "label": "Random Words",
    "description": "Random words",
    "example": "hello world foo",
    "category": "Text",
    "args": [
      {
        "name": "count",
        "description": "Number of words",
        "type": "number",
        "optional": true,
        "defaultValue": "3",
        "example": "5",
        "order": 0
      }
    ]
  },
  {
    "name": "$randomAbbreviation",
    "label": "Random Abbreviation",
    "description": "Random hacker abbreviation",
    "example": "HTTP",
    "category": "Text"
  },
  {
    "name": "$randomHexColor",
    "label": "Random Hex Color",
    "description": "Random hex color code",
    "example": "#ff5733",
    "category": "Color"
  },
  {
    "name": "$randomRgbColor",
    "label": "Random RGB Color",
    "description": "Random RGB color value",
    "example": "rgb(255, 87, 51)",
    "category": "Color"
  },
  {
    "name": "$randomColorName",
    "label": "Random Color Name",
    "description": "Random human-readable color name",
    "example": "red",
    "category": "Color"
  },
  {
    "name": "$randomColor",
    "label": "Random Color (Alias)",
    "description": "Random human-readable color name (alias for $randomColorName)",
    "example": "red",
    "category": "Color"
  },
  {
    "name": "$randomBoolean",
    "label": "Random Boolean",
    "description": "Random true or false",
    "example": "true",
    "category": "Data",
    "args": [
      {
        "name": "probability",
        "description": "Probability of true (0-1)",
        "type": "number",
        "optional": true,
        "defaultValue": "0.5",
        "example": "0.8",
        "order": 0
      }
    ]
  },
  {
    "name": "$randomDateFuture",
    "label": "Random Future Date",
    "description": "Random future date (ISO string)",
    "example": "2025-12-01T14:00:00.000Z",
    "category": "Data",
    "args": [
      {
        "name": "years",
        "description": "Max years in the future",
        "type": "number",
        "optional": true,
        "defaultValue": "1",
        "example": "5",
        "order": 0
      }
    ]
  },
  {
    "name": "$randomDatePast",
    "label": "Random Past Date",
    "description": "Random past date (ISO string)",
    "example": "2022-06-15T08:00:00.000Z",
    "category": "Data",
    "args": [
      {
        "name": "years",
        "description": "Max years in the past",
        "type": "number",
        "optional": true,
        "defaultValue": "1",
        "example": "10",
        "order": 0
      }
    ]
  },
  {
    "name": "$randomDateRecent",
    "label": "Random Recent Date",
    "description": "Random recent date (ISO string)",
    "example": "2024-03-23T10:30:00.000Z",
    "category": "Data",
    "args": [
      {
        "name": "days",
        "description": "Max days in the past",
        "type": "number",
        "optional": true,
        "defaultValue": "1",
        "example": "7",
        "order": 0
      }
    ]
  },
  {
    "name": "$randomWeekday",
    "label": "Random Weekday",
    "description": "Random day of the week",
    "example": "Monday",
    "category": "Data",
    "args": [
      {
        "name": "abbreviated",
        "description": "Use abbreviated form (Mon, Tue, ...)",
        "type": "boolean",
        "optional": true,
        "defaultValue": "false",
        "example": "true",
        "order": 0
      }
    ]
  },
  {
    "name": "$randomMonth",
    "label": "Random Month",
    "description": "Random month name",
    "example": "January",
    "category": "Data",
    "args": [
      {
        "name": "abbreviated",
        "description": "Use abbreviated form (Jan, Feb, ...)",
        "type": "boolean",
        "optional": true,
        "defaultValue": "false",
        "example": "true",
        "order": 0
      }
    ]
  },
  {
    "name": "$randomDatabaseColumn",
    "label": "Random Database Column",
    "description": "Random database column name",
    "example": "id",
    "category": "Data"
  },
  {
    "name": "$randomDatabaseType",
    "label": "Random Database Type",
    "description": "Random database data type",
    "example": "varchar",
    "category": "Data"
  },
  {
    "name": "$randomDatabaseCollation",
    "label": "Random Database Collation",
    "description": "Random database collation",
    "example": "utf8_general_ci",
    "category": "Data"
  },
  {
    "name": "$randomDatabaseEngine",
    "label": "Random Database Engine",
    "description": "Random database engine",
    "example": "InnoDB",
    "category": "Data"
  },
  {
    "name": "$randomFileName",
    "label": "Random File Name",
    "description": "Random file name with extension",
    "example": "report.pdf",
    "category": "System"
  },
  {
    "name": "$randomFileType",
    "label": "Random File Type",
    "description": "Random file type",
    "example": "video",
    "category": "System"
  },
  {
    "name": "$randomFileExt",
    "label": "Random File Extension",
    "description": "Random file extension",
    "example": "pdf",
    "category": "System",
    "args": [
      {
        "name": "mimeType",
        "description": "MIME type to derive extension from",
        "type": "string",
        "optional": true,
        "example": "application/json",
        "order": 0
      }
    ]
  },
  {
    "name": "$randomCommonFileName",
    "label": "Random Common File Name",
    "description": "Random common file name",
    "example": "document.pdf",
    "category": "System"
  },
  {
    "name": "$randomCommonFileType",
    "label": "Random Common File Type",
    "description": "Random common file type",
    "example": "application",
    "category": "System"
  },
  {
    "name": "$randomCommonFileExt",
    "label": "Random Common File Ext",
    "description": "Random common file extension",
    "example": "pdf",
    "category": "System"
  },
  {
    "name": "$randomFilePath",
    "label": "Random File Path",
    "description": "Random file path",
    "example": "/usr/local/bin/report.pdf",
    "category": "System"
  },
  {
    "name": "$randomDirectoryPath",
    "label": "Random Directory Path",
    "description": "Random directory path",
    "example": "/usr/local/bin",
    "category": "System"
  },
  {
    "name": "$randomMimeType",
    "label": "Random MIME Type",
    "description": "Random MIME type",
    "example": "application/json",
    "category": "System"
  },
  {
    "name": "$randomSemver",
    "label": "Random Semver",
    "description": "Random semantic version",
    "example": "1.2.3",
    "category": "System"
  },
  {
    "name": "$randomPrice",
    "label": "Random Price",
    "description": "Random price value",
    "example": "29.99",
    "category": "Commerce",
    "args": [
      {
        "name": "min",
        "description": "Minimum price",
        "type": "number",
        "optional": true,
        "defaultValue": "1",
        "example": "10",
        "order": 0
      },
      {
        "name": "max",
        "description": "Maximum price",
        "type": "number",
        "optional": true,
        "defaultValue": "1000",
        "example": "100",
        "order": 1
      },
      {
        "name": "dec",
        "description": "Decimal places",
        "type": "number",
        "optional": true,
        "defaultValue": "2",
        "example": "0",
        "order": 2
      },
      {
        "name": "symbol",
        "description": "Currency symbol prefix",
        "type": "string",
        "optional": true,
        "example": "$",
        "order": 3
      }
    ]
  },
  {
    "name": "$randomProduct",
    "label": "Random Product",
    "description": "Random product type",
    "example": "Keyboard",
    "category": "Commerce"
  },
  {
    "name": "$randomProductName",
    "label": "Random Product Name",
    "description": "Random product name",
    "example": "Ergonomic Keyboard",
    "category": "Commerce"
  },
  {
    "name": "$randomProductAdjective",
    "label": "Random Product Adjective",
    "description": "Random product adjective",
    "example": "Ergonomic",
    "category": "Commerce"
  },
  {
    "name": "$randomProductMaterial",
    "label": "Random Product Material",
    "description": "Random product material",
    "example": "Steel",
    "category": "Commerce"
  },
  {
    "name": "$randomDepartment",
    "label": "Random Department",
    "description": "Random department name",
    "example": "Electronics",
    "category": "Commerce"
  },
  {
    "name": "$randomCompanyName",
    "label": "Random Company",
    "description": "Random company name",
    "example": "Acme Corp",
    "category": "Commerce"
  },
  {
    "name": "$randomCompanySuffix",
    "label": "Random Company Suffix",
    "description": "Random company suffix",
    "example": "Inc",
    "category": "Business"
  },
  {
    "name": "$randomBs",
    "label": "Random BS",
    "description": "Random business buzz phrase",
    "example": "synergize scalable supply-chains",
    "category": "Business"
  },
  {
    "name": "$randomBsAdjective",
    "label": "Random BS Adjective",
    "description": "Random business buzz adjective",
    "example": "scalable",
    "category": "Business"
  },
  {
    "name": "$randomBsBuzz",
    "label": "Random BS Buzz",
    "description": "Random business buzz verb",
    "example": "synergize",
    "category": "Business"
  },
  {
    "name": "$randomBsNoun",
    "label": "Random BS Noun",
    "description": "Random business buzz noun",
    "example": "supply-chains",
    "category": "Business"
  },
  {
    "name": "$randomCatchPhrase",
    "label": "Random Catch Phrase",
    "description": "Random company catch phrase",
    "example": "Multi-layered client-server neural-net",
    "category": "Business"
  },
  {
    "name": "$randomCatchPhraseAdjective",
    "label": "Random Catch Phrase Adjective",
    "description": "Random catch phrase adjective",
    "example": "Multi-layered",
    "category": "Business"
  },
  {
    "name": "$randomCatchPhraseDescriptor",
    "label": "Random Catch Phrase Descriptor",
    "description": "Random catch phrase descriptor",
    "example": "client-server",
    "category": "Business"
  },
  {
    "name": "$randomCatchPhraseNoun",
    "label": "Random Catch Phrase Noun",
    "description": "Random catch phrase noun",
    "example": "neural-net",
    "category": "Business"
  },
  {
    "name": "$randomAvatarImage",
    "label": "Random Avatar Image",
    "description": "Random avatar image URL",
    "example": "https://avatars.githubusercontent.com/u/12345",
    "category": "Images"
  },
  {
    "name": "$randomImageUrl",
    "label": "Random Image URL",
    "description": "Random image URL",
    "example": "https://loremflickr.com/640/480",
    "category": "Images"
  },
  {
    "name": "$randomAbstractImage",
    "label": "Random Abstract Image",
    "description": "Random abstract image URL",
    "example": "https://loremflickr.com/640/480/abstract",
    "category": "Images"
  },
  {
    "name": "$randomAnimalsImage",
    "label": "Random Animals Image",
    "description": "Random animals image URL",
    "example": "https://loremflickr.com/640/480/animals",
    "category": "Images"
  },
  {
    "name": "$randomBusinessImage",
    "label": "Random Business Image",
    "description": "Random business image URL",
    "example": "https://loremflickr.com/640/480/business",
    "category": "Images"
  },
  {
    "name": "$randomCatsImage",
    "label": "Random Cats Image",
    "description": "Random cats image URL",
    "example": "https://loremflickr.com/640/480/cats",
    "category": "Images"
  },
  {
    "name": "$randomCityImage",
    "label": "Random City Image",
    "description": "Random city image URL",
    "example": "https://loremflickr.com/640/480/city",
    "category": "Images"
  },
  {
    "name": "$randomFoodImage",
    "label": "Random Food Image",
    "description": "Random food image URL",
    "example": "https://loremflickr.com/640/480/food",
    "category": "Images"
  },
  {
    "name": "$randomNightlifeImage",
    "label": "Random Nightlife Image",
    "description": "Random nightlife image URL",
    "example": "https://loremflickr.com/640/480/nightlife",
    "category": "Images"
  },
  {
    "name": "$randomFashionImage",
    "label": "Random Fashion Image",
    "description": "Random fashion image URL",
    "example": "https://loremflickr.com/640/480/fashion",
    "category": "Images"
  },
  {
    "name": "$randomPeopleImage",
    "label": "Random People Image",
    "description": "Random people image URL",
    "example": "https://loremflickr.com/640/480/people",
    "category": "Images"
  },
  {
    "name": "$randomNatureImage",
    "label": "Random Nature Image",
    "description": "Random nature image URL",
    "example": "https://loremflickr.com/640/480/nature",
    "category": "Images"
  },
  {
    "name": "$randomSportsImage",
    "label": "Random Sports Image",
    "description": "Random sports image URL",
    "example": "https://loremflickr.com/640/480/sports",
    "category": "Images"
  },
  {
    "name": "$randomTransportImage",
    "label": "Random Transport Image",
    "description": "Random transport image URL",
    "example": "https://loremflickr.com/640/480/transport",
    "category": "Images"
  },
  {
    "name": "$randomImageDataUri",
    "label": "Random Image Data URI",
    "description": "Random image as data URI",
    "example": "data:image/svg+xml;charset=UTF-8,...",
    "category": "Images"
  },
  {
    "name": "$randomNoun",
    "label": "Random Noun",
    "description": "Random noun",
    "example": "bus",
    "category": "Words"
  },
  {
    "name": "$randomVerb",
    "label": "Random Verb",
    "description": "Random verb",
    "example": "navigate",
    "category": "Words"
  },
  {
    "name": "$randomIngverb",
    "label": "Random Gerund Verb",
    "description": "Random verb in gerund form (ending in -ing)",
    "example": "running",
    "category": "Words"
  },
  {
    "name": "$randomAdjective",
    "label": "Random Adjective",
    "description": "Random adjective",
    "example": "electronic",
    "category": "Words"
  },
  {
    "name": "$randomWord",
    "label": "Random Word",
    "description": "Random word",
    "example": "pixel",
    "category": "Words"
  },
  {
    "name": "$randomPhrase",
    "label": "Random Phrase",
    "description": "Random hacker-style phrase",
    "example": "If we program the firewall, we can get to the SQL interface through the neural TCP port!",
    "category": "Words"
  },
  {
    "name": "$randomLoremWords",
    "label": "Random Lorem Words",
    "description": "Random lorem ipsum words",
    "example": "dolor sit amet",
    "category": "Words"
  },
  {
    "name": "$randomLoremSentences",
    "label": "Random Lorem Sentences",
    "description": "Random lorem ipsum sentences",
    "example": "Lorem ipsum dolor sit amet.",
    "category": "Words"
  },
  {
    "name": "$randomLoremParagraphs",
    "label": "Random Lorem Paragraphs",
    "description": "Random lorem ipsum paragraphs",
    "example": "Lorem ipsum dolor sit amet...",
    "category": "Words"
  },
  {
    "name": "$randomLoremText",
    "label": "Random Lorem Text",
    "description": "Random lorem ipsum text",
    "example": "Lorem ipsum dolor sit amet...",
    "category": "Words"
  },
  {
    "name": "$randomLoremLines",
    "label": "Random Lorem Lines",
    "description": "Random lorem ipsum lines",
    "example": "Lorem ipsum dolor sit amet\nconsectetur adipiscing elit",
    "category": "Words"
  }
] as const;
