#pragma once

// This is the intentionally small M14 generated-product scaffold. Device
// commissioning metadata and application communication objects are generated
// from the XML product in a later packaging step; no LogikSmith endpoint is
// represented as an ETS GroupObject.
#define MAIN_FirmwareName "LogikSmith"
#define MAIN_FirmwareRevision 0
#define MAIN_OpenKnxId 0xAC
#define MAIN_ApplicationNumber 19
#define MAIN_ApplicationVersion 1
#define MAIN_ApplicationEncoding iso-8859-15
#define MAIN_OrderNumber "LogikSmith"

// M14 has no generated BASE parameter block yet. Keep OpenKNX's watchdog
// guard deterministic until the ETS product packaging is added.
#define ParamBASE_Watchdog 0

// The common runtime compiles a few BASE services even when the product does
// not expose their ETS parameters. These constants keep those services
// disabled in the M14 firmware scaffold; generated product metadata will
// replace them when commissioning packaging is introduced.
#define ParamBASE_Info1LedFunc 0
#define ParamBASE_Info2LedFunc 0
#define ParamBASE_Info3LedFunc 0
#define ParamBASE_CombinedTimeDate 0
#define ParamBASE_SummertimeAll 0
#define ParamBASE_Timezone 0
#define ParamBASE_TimezoneCustom "UTC"
#define ParamBASE_ReadTimeDate 0
#define ParamBASE_InternalTime 0
