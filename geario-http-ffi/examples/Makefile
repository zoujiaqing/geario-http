# Builds the C examples against the release staticlib.
#
#   cargo build -p geario-http-ffi --release
#   make -C geario-http-ffi/examples

TARGET_DIR ?= ../../target/release
CFLAGS += -I../include -Wall -Wextra
LDLIBS = $(TARGET_DIR)/libgeario_http_ffi.a

UNAME := $(shell uname -s)
ifeq ($(UNAME),Darwin)
LDLIBS += -framework CoreFoundation -framework Security -lc++
else
LDLIBS += -lpthread -ldl -lm
endif

all: version server stream

version: version.c
	$(CC) $(CFLAGS) -o $@ $< $(LDLIBS)

server: server.c
	$(CC) $(CFLAGS) -o $@ $< $(LDLIBS)

stream: stream.c
	$(CC) $(CFLAGS) -o $@ $< $(LDLIBS)

clean:
	rm -f version server stream

.PHONY: all clean
