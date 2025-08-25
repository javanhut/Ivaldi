package logging

import (
	"fmt"
	"log"
	"strings"
	"time"
)

// LogLevel represents the logging level
type LogLevel int

const (
	DEBUG LogLevel = iota
	INFO
	WARN
	ERROR
)

var levelNames = map[LogLevel]string{
	DEBUG: "DEBUG",
	INFO:  "INFO",
	WARN:  "WARN",
	ERROR: "ERROR",
}

// Logger provides structured logging functionality
type Logger struct {
	level  LogLevel
	prefix string
}

// NewLogger creates a new logger with the specified level and prefix
func NewLogger(level LogLevel, prefix string) *Logger {
	return &Logger{
		level:  level,
		prefix: prefix,
	}
}

// SetLevel changes the logging level
func (l *Logger) SetLevel(level LogLevel) {
	l.level = level
}

// shouldLog checks if the message should be logged at the current level
func (l *Logger) shouldLog(level LogLevel) bool {
	return level >= l.level
}

// format formats a log message with timestamp, level, prefix, and structured key/value pairs
func (l *Logger) format(level LogLevel, message string, args ...interface{}) string {
	timestamp := time.Now().Format("2006-01-02 15:04:05")
	levelStr := levelNames[level]

	// Build the base message
	var baseMessage string
	if l.prefix != "" {
		baseMessage = fmt.Sprintf("[%s] %s [%s] %s", timestamp, levelStr, l.prefix, message)
	} else {
		baseMessage = fmt.Sprintf("[%s] %s %s", timestamp, levelStr, message)
	}

	// Process structured key/value pairs from args
	if len(args) > 0 {
		var kvPairs []string

		// Process args in pairs (key, value)
		for i := 0; i < len(args); i += 2 {
			if i+1 < len(args) {
				// We have a key-value pair
				key := fmt.Sprintf("%v", args[i])
				value := fmt.Sprintf("%v", args[i+1])
				kvPairs = append(kvPairs, fmt.Sprintf("%s=%s", key, value))
			} else {
				// Odd number of args - handle the remaining single value
				key := fmt.Sprintf("%v", args[i])
				kvPairs = append(kvPairs, fmt.Sprintf("%s=<missing_value>", key))
			}
		}

		// Join key/value pairs and append to base message
		if len(kvPairs) > 0 {
			baseMessage += " " + strings.Join(kvPairs, " ")
		}
	}

	return baseMessage
}

// Debug logs a debug message
func (l *Logger) Debug(message string, args ...interface{}) {
	if l.shouldLog(DEBUG) {
		log.Println(l.format(DEBUG, message, args...))
	}
}

// Info logs an info message
func (l *Logger) Info(message string, args ...interface{}) {
	if l.shouldLog(INFO) {
		log.Println(l.format(INFO, message, args...))
	}
}

// Warn logs a warning message
func (l *Logger) Warn(message string, args ...interface{}) {
	if l.shouldLog(WARN) {
		log.Println(l.format(WARN, message, args...))
	}
}

// Error logs an error message
func (l *Logger) Error(message string, args ...interface{}) {
	if l.shouldLog(ERROR) {
		log.Println(l.format(ERROR, message, args...))
	}
}

// WithFields creates a new logger with additional context fields
func (l *Logger) WithFields(fields map[string]interface{}) *Logger {
	// For now, just return the same logger
	// In a more sophisticated implementation, this would add fields to the context
	return l
}

// WithPrefix creates a new logger with a different prefix
func (l *Logger) WithPrefix(prefix string) *Logger {
	return &Logger{
		level:  l.level,
		prefix: prefix,
	}
}

// Global logger instance
var defaultLogger = NewLogger(INFO, "")

// SetDefaultLevel sets the default logging level
func SetDefaultLevel(level LogLevel) {
	defaultLogger.SetLevel(level)
}

// Debug logs a debug message using the default logger
func Debug(message string, args ...interface{}) {
	defaultLogger.Debug(message, args...)
}

// Info logs an info message using the default logger
func Info(message string, args ...interface{}) {
	defaultLogger.Info(message, args...)
}

// Warn logs a warning message using the default logger
func Warn(message string, args ...interface{}) {
	defaultLogger.Warn(message, args...)
}

// Error logs an error message using the default logger
func Error(message string, args ...interface{}) {
	defaultLogger.Error(message, args...)
}

// WithFields creates a new logger with additional context fields using the default logger
func WithFields(fields map[string]interface{}) *Logger {
	return defaultLogger.WithFields(fields)
}

// WithPrefix creates a new logger with a different prefix using the default logger
func WithPrefix(prefix string) *Logger {
	return defaultLogger.WithPrefix(prefix)
}
