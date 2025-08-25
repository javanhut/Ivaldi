package logging

import (
	"fmt"
	"log"
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

// format formats a log message with timestamp, level, and prefix
func (l *Logger) format(level LogLevel, message string, args ...interface{}) string {
	timestamp := time.Now().Format("2006-01-02 15:04:05")
	levelStr := levelNames[level]

	if l.prefix != "" {
		return fmt.Sprintf("[%s] %s [%s] %s", timestamp, levelStr, l.prefix, fmt.Sprintf(message, args...))
	}
	return fmt.Sprintf("[%s] %s %s", timestamp, levelStr, fmt.Sprintf(message, args...))
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
