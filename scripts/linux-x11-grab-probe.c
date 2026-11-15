#define _POSIX_C_SOURCE 200809L

#include <sys/types.h>

#include <X11/Xlib.h>
#include <X11/Xproto.h>
#include <X11/extensions/XI2proto.h>
#include <X11/extensions/XIproto.h>
#include <X11/extensions/XRes.h>
#include <X11/extensions/record.h>

#include <errno.h>
#include <inttypes.h>
#include <poll.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

struct observer {
    Display *control;
    int input_opcode;
    Atom moveresize_atom;
};

static const char *request_name(const struct observer *observer,
                                const unsigned char *request) {
    if (request[0] == X_GrabKeyboard) return "GrabKeyboard";
    if (request[0] == X_UngrabKeyboard) return "UngrabKeyboard";
    if (request[0] != observer->input_opcode) return NULL;

    switch (request[1]) {
        case X_GrabDevice: return "GrabDevice";
        case X_UngrabDevice: return "UngrabDevice";
        case X_XIGrabDevice: return "XIGrabDevice";
        case X_XIUngrabDevice: return "XIUngrabDevice";
        default: return NULL;
    }
}

static pid_t client_pid(Display *control, XID client) {
    XResClientIdSpec spec = { client, XRES_CLIENT_ID_PID_MASK };
    XResClientIdValue *ids = NULL;
    long count = 0;
    pid_t pid = -1;
    if (XResQueryClientIds(control, 1, &spec, &count, &ids) != Success) return pid;
    for (long i = 0; i < count && pid == -1; ++i) {
        pid = XResGetClientPid(&ids[i]);
    }
    XResClientIdsDestroy(count, ids);
    return pid;
}

static bool read_moveresize(const struct observer *observer,
                            const XRecordInterceptData *item, xSendEventReq *message) {
    if (item->data[0] != X_SendEvent || item->client_swapped ||
        item->data_len < sizeof(*message) / 4) return false;
    memcpy(message, item->data, sizeof(*message));
    return message->event.u.u.type == ClientMessage && message->event.u.u.detail == 32 &&
           message->event.u.clientMessage.u.l.type == observer->moveresize_atom;
}

static void report_request(XPointer closure, XRecordInterceptData *item) {
    struct observer *observer = (struct observer *)closure;
    if (item->category == XRecordFromClient && item->data_len > 0) {
        const char *name = request_name(observer, item->data);
        xSendEventReq message;
        bool moveresize = read_moveresize(observer, item, &message);
        struct timespec now;
        if ((name || moveresize) && clock_gettime(CLOCK_MONOTONIC, &now) == 0) {
            uint64_t timestamp = (uint64_t)now.tv_sec * 1000000000 + (uint64_t)now.tv_nsec;
            pid_t pid = client_pid(observer->control, item->id_base);
            char command[32] = "unknown";
            char path[64];
            snprintf(path, sizeof(path), "/proc/%ld/comm", (long)pid);
            FILE *file = pid > 0 ? fopen(path, "r") : NULL;
            if (file) {
                if (fgets(command, sizeof(command), file)) command[strcspn(command, "\r\n")] = '\0';
                fclose(file);
            }
            if (moveresize) {
                printf("MimiLinuxX11MoveResize mono_ns=%" PRIu64 " window=0x%lx direction=%ld pid=%ld process=%s\n",
                       timestamp, (unsigned long)message.event.u.clientMessage.window,
                       (long)message.event.u.clientMessage.u.l.longs2, (long)pid, command);
            } else {
                printf("MimiLinuxX11Grab mono_ns=%" PRIu64 " request=%s client=0x%lx pid=%ld process=%s\n",
                       timestamp, name, item->id_base, (long)pid, command);
            }
        }
    }
    XRecordFreeData(item);
}

static int exercise_grab(Display *display) {
    Window root = DefaultRootWindow(display);
    int result = XGrabKeyboard(display, root, False,
                               GrabModeAsync, GrabModeAsync, CurrentTime);
    if (result != GrabSuccess) return 1;
    XUngrabKeyboard(display, CurrentTime);
    // There is no managed window to move. Verify decoding with a cancel message.
    XEvent event = {0};
    event.xclient.type = ClientMessage;
    event.xclient.window = root;
    event.xclient.message_type = XInternAtom(display, "_NET_WM_MOVERESIZE", False);
    event.xclient.format = 32;
    event.xclient.data.l[2] = 11;
    if (!XSendEvent(display, root, False, SubstructureNotifyMask, &event)) return 1;
    XSync(display, False);
    // Remain connected until the observer has resolved this client's PID.
    return getchar() == '\n' ? 0 : 1;
}

int main(int argc, char **argv) {
    // This executable is called only inside the isolated Xvfb package test.
    // Report only grab requests and move/resize metadata from synthetic windows.
    const char *isolated = getenv("MIMI_LINUX_SMOKE_SESSION");
    if (!isolated || strcmp(isolated, "1") != 0) return 1;
    setvbuf(stdout, NULL, _IOLBF, 0);
    Display *control = XOpenDisplay(NULL);
    if (!control) return 1;
    if (argc == 2 && strcmp(argv[1], "--exercise") == 0) return exercise_grab(control);
    if (argc != 1) return 1;
    Display *data = XOpenDisplay(NULL);
    int major, minor, opcode, ignored;
    if (!data || !XRecordQueryVersion(control, &major, &minor) ||
        !XResQueryVersion(control, &major, &minor) || (major == 1 && minor < 2) ||
        !XQueryExtension(control, "XInputExtension", &opcode, &ignored, &ignored)) {
        fputs("Could not initialize X11 RECORD, XRes or XInput\n", stderr);
        return 1;
    }

    XRecordRange core = {0};
    core.core_requests.first = X_GrabKeyboard;
    core.core_requests.last = X_UngrabKeyboard;
    XRecordRange send = {0};
    send.core_requests.first = send.core_requests.last = X_SendEvent;
    XRecordRange xi1 = {0};
    xi1.ext_requests.ext_major.first = xi1.ext_requests.ext_major.last = opcode;
    xi1.ext_requests.ext_minor.first = X_GrabDevice;
    xi1.ext_requests.ext_minor.last = X_UngrabDevice;
    XRecordRange xi2 = xi1;
    xi2.ext_requests.ext_minor.first = X_XIGrabDevice;
    xi2.ext_requests.ext_minor.last = X_XIUngrabDevice;
    XRecordRange *ranges[] = { &core, &send, &xi1, &xi2 };
    XRecordClientSpec clients = XRecordAllClients;
    XRecordContext context = XRecordCreateContext(control, 0, &clients, 1, ranges, 4);
    XSync(control, False);
    struct observer observer = { control, opcode, XInternAtom(control, "_NET_WM_MOVERESIZE", False) };
    if (!context || !XRecordEnableContextAsync(data, context, report_request, (XPointer)&observer)) {
        fputs("Could not enable X11 grab request observation\n", stderr);
        return 1;
    }

    puts("MimiLinuxX11GrabReady");
    struct pollfd input = { ConnectionNumber(data), POLLIN, 0 };
    while (1) {
        int ready = poll(&input, 1, -1);
        if (ready < 0 && errno == EINTR) continue;
        if (ready <= 0 || (input.revents & (POLLERR | POLLHUP | POLLNVAL))) return 1;
        XRecordProcessReplies(data);
    }
}
