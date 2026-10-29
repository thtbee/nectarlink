// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once
#include <stddef.h>
#include <stdint.h>

#define NL_CAMERA_DEVICE_UUID "3ACD83C9-37BB-566E-99B5-CC32208B3E27"
#define NL_CAMERA_SOURCE_UUID "BFB8454A-52BE-5130-B3F6-0E8D4A3DCBAE"
#define NL_CAMERA_SINK_UUID "7A39CA84-50D7-59EC-AA23-B4BC7A686487"
#define NL_CAMERA_HOST_ID "app.nectarlink.desktop"
#define NL_CAMERA_EXTENSION_ID "app.nectarlink.desktop.CameraExtension"
#define NL_CAMERA_WIDTH 3840
#define NL_CAMERA_HEIGHT 2160
#define NL_CAMERA_FPS 30
#define NL_CAMERA_MAX_AGE_NS UINT64_C(250000000)

#ifdef __cplusplus
extern "C" {
#endif
typedef struct NLCameraProducer NLCameraProducer;
enum { NL_CAMERA_OK = 0, NL_CAMERA_DROPPED = 1, NL_CAMERA_INVALID = -1,
       NL_CAMERA_NOT_INSTALLED = -2, NL_CAMERA_OS_ERROR = -3 };

// Open only after a local Start/phone-accepted session has an exclusive
// (device ID, stream generation) owner. No phone/permission/activation side effects.
// A published device UID alone is not proof of approved installation/signing.
int32_t nl_camera_is_registered(void);
int32_t nl_camera_open(NLCameraProducer **out);
// One serialized calling lane per handle. Tight BGRX comes directly from
// webcam.rs's existing to_bgrx + optional mirror step. Copies before returning.
// Aspect-fits to the extension's fixed 3840x2160 BGRA sink format.
int32_t nl_camera_push_bgrx(NLCameraProducer *producer, const uint8_t *bytes,
                          size_t length, uint32_t width, uint32_t height);
// Stop the sink before relinquishing the capture owner. Never keep a producer
// alive across an owner/peer change. Calling close(NULL) is harmless.
void nl_camera_close(NLCameraProducer *producer);
#ifdef __cplusplus
}
#endif
