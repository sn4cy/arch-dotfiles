#include <sys/socket.h>
#include <sys/un.h>
#include <sys/time.h>
#include <unistd.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(int argc, char **argv) {
    const char *runtime = getenv("XDG_RUNTIME_DIR");
    if (argc != 2 || !runtime) return 2;
    struct sockaddr_un addr = {.sun_family = AF_UNIX};
    if (snprintf(addr.sun_path, sizeof(addr.sun_path), "%s/fast-alt-tab.sock", runtime) >= (int)sizeof(addr.sun_path)) return 2;
    int fd = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (fd < 0) { perror("socket"); return 1; }
    struct timeval timeout = {.tv_sec = 1};
    setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &timeout, sizeof(timeout));
    if (connect(fd, (struct sockaddr *)&addr, sizeof(addr)) < 0) { perror("connect"); close(fd); return 1; }
    char command[64];
    int length = snprintf(command, sizeof(command), "%s\n", argv[1]);
    if (length < 0 || length >= (int)sizeof(command) || send(fd, command, length, MSG_NOSIGNAL) != length) { close(fd); return 1; }
    char reply[4096];
    ssize_t count = read(fd, reply, sizeof(reply));
    close(fd);
    if (count <= 0) return 1;
    if (!strcmp(argv[1], "status")) fwrite(reply, 1, count, stdout);
    return 0;
}
