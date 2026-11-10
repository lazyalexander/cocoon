/* Freestanding Linux ARM64 PID 1. No libc, shell, dynamic loader or daemons.
 * A fixed acceptance workload, not an agent or a production guest runtime. */
typedef unsigned long usize;
static long call(long n, long a, long b, long c, long d) {
    register long x0 __asm__("x0") = a;
    register long x1 __asm__("x1") = b;
    register long x2 __asm__("x2") = c;
    register long x3 __asm__("x3") = d;
    register long x8 __asm__("x8") = n;
    __asm__ volatile("svc #0" : "+r"(x0) : "r"(x1), "r"(x2), "r"(x3), "r"(x8) : "memory");
    return x0;
}
static long read_file(const char *path, char *buf, long size) {
    long fd = call(56, -100, (long)path, 0, 0); /* openat */
    if (fd < 0) return fd;
    long count = call(63, fd, (long)buf, size, 0);
    call(57, fd, 0, 0, 0);
    return count;
}
static int output;
static void write_all(const char *buf, long size) {
    while (size > 0) {
        long n = call(64, output, (long)buf, size, 0);
        if (n <= 0) return;
        buf += n;
        size -= n;
    }
}
static void marker(const char *label, long length, const char *token) {
    write_all(label, length);
    write_all(token, 32);
    write_all("\n", 1);
}
__attribute__((noreturn)) void _start(void) {
    char token[32], input[4096], mode[1];
    output = (int)call(56, -100, (long)"/dev/hvc0", 2, 0);
    if (output < 0) output = (int)call(56, -100, (long)"/dev/console", 2, 0);
    if (read_file("/token", token, sizeof(token)) == 32) {
        marker("COCOON_READY:", 13, token);
        long count = read_file("/input", input, sizeof(input));
        long has_mode = read_file("/mode", mode, 1);
        if (has_mode == 1 && mode[0] == 'l') {
            char command, value = '0', saved = '0';
            while (call(63, output, (long)&command, 1, 0) == 1) {
                if (command == 'A' && value < '9') value++;
                else if (command == 'S') saved = value;
                else if (command == 'R') value = saved;
                else continue;
                write_all("COCOON_STATE:", 13);
                write_all(token, 32);
                write_all(&command, 1);
                write_all(&value, 1);
                write_all("\n", 1);
            }
        }
        if (has_mode == 1 && mode[0] == 'r' && count >= 0) {
            for (long i = 0; i < count; i++)
                if (input[i] >= 'a' && input[i] <= 'z') input[i] -= 'a' - 'A';
            marker("COCOON_OUTPUT:", 14, token);
            write_all(input, count);
            write_all("\n", 1);
            marker("COCOON_DONE:", 12, token);
        }
    }
    /* Stay alive so the host must actually stop the VM, even after success. */
    const long pause[2] = {1, 0};
    for (;;) call(101, (long)pause, 0, 0, 0); /* nanosleep */
}
