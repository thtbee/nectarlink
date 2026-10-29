// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
enum { NL_CAMERA_ACTIVATION_SUBMITTED = 0, NL_CAMERA_ACTIVATION_NEEDS_APPROVAL = 1,
       NL_CAMERA_ACTIVATION_COMPLETED = 2, NL_CAMERA_ACTIVATION_REBOOT = 3,
       NL_CAMERA_ACTIVATION_FAILED = 4, NL_CAMERA_ACTIVATION_BUSY = 5 };
typedef void (*NLCameraActivationCallback)(void *context, int32_t status, int64_t os_error);
// Explicit UI action only. Callback runs on main queue. Context must remain
// valid until terminal COMPLETED/REBOOT/FAILED/BUSY. Submitted/pending are not Ready.
// COMPLETED still requires a published-device readiness probe before capability advertisement.
int32_t nl_camera_payload_valid(void);
void nl_camera_request_activation(int32_t enable, NLCameraActivationCallback callback, void *context);
#ifdef __cplusplus
}
#endif
