#include "OpenKNX.h"

#include "knxprod.h"
#include "logiksmith_openknx/logic_smith_module.h"

void setup() {
    openknx.init(MAIN_FirmwareRevision);
    // Keep the module id in the application-owned range. Other OpenKNX
    // modules can still be registered beside LogicSmith in the same image.
    openknx.addModule(10, logiksmith::openknx::logicSmithModule);
    openknx.setup();
}

void loop() {
    openknx.loop();
}
