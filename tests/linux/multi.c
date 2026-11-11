// fork + exec-less child + pipe + waitpid + pthreads with a mutex/condvar (futex) + thread join
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <pthread.h>
#include <sys/wait.h>
static pthread_mutex_t m = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t cv = PTHREAD_COND_INITIALIZER;
static int counter, go;
static void *worker(void *arg) {
    pthread_mutex_lock(&m);
    while (!go) pthread_cond_wait(&cv, &m);
    pthread_mutex_unlock(&m);
    for (int i = 0; i < 1000; i++) { pthread_mutex_lock(&m); counter += (int)(long)arg; pthread_mutex_unlock(&m); }
    return arg;
}
int main(int argc, char **argv) {
    if (argc > 1) { printf("exec child %s\n", argv[1]); return 7; }
    int fd[2]; pipe(fd);
    pid_t pid = fork();
    if (!pid) { close(fd[0]); write(fd[1], "ping", 4); _exit(5); }
    char buf[8] = {0}; close(fd[1]);
    int n = read(fd[0], buf, 8), st = 0;
    waitpid(pid, &st, 0);
    printf("child said %s (%d) exit %d\n", buf, n, WEXITSTATUS(st));
    pthread_t t[3];
    for (long i = 0; i < 3; i++) pthread_create(&t[i], 0, worker, (void *)(i + 1));
    usleep(1000);
    pthread_mutex_lock(&m); go = 1; pthread_cond_broadcast(&cv); pthread_mutex_unlock(&m);
    long sum = 0; void *r;
    for (int i = 0; i < 3; i++) { pthread_join(t[i], &r); sum += (long)r; }
    printf("threads counter %d sum %ld\n", counter, sum);
    fflush(stdout);
    pid = fork();
    if (!pid) { execl("/bin/app", "/bin/app", "x", (char *)0); _exit(99); }
    waitpid(pid, &st, 0);
    printf("exec exit %d\n", WEXITSTATUS(st));
    return 0;
}
